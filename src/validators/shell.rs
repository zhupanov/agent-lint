//! Shell and awk lexical analysis shared by the G009-G011 shipped-script
//! contracts.
//!
//! This is the single lexical layer the portability rules build on. Instead of
//! matching regexes against whole raw lines (which conflates comments, quoted
//! text, expansions, and executable code), the scanner walks a logical line
//! once and produces a byte-offset-preserving *masked* projection: comment
//! text, single-quoted text, ANSI-C `$'...'` text, and the literal portions of
//! double-quoted text are blanked to spaces, while executable code and live
//! expansions (`${...}`, `$(...)`, `$((...))`, backticks) are preserved. The
//! rule helpers then reason about that projection, so a construct inside a
//! comment or an inert string can never be mistaken for live code, and a live
//! construct inside a double-quoted expansion is never lost.
//!
//! The module reports facts only; it never emits diagnostics or touches process
//! state (see `ARCHITECTURAL_GUIDELINES.md` G-Layer-1).

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// A byte range within a logical line, `start..end`, end-exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
}

/// A Bash-4+ construct detected in live code, with a stable human label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Construct {
    pub label: &'static str,
    pub span: Span,
}

/// Lexical enablement of the shell options that gate two G010 hazards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SetFlags {
    pub errexit: bool,
    pub nounset: bool,
}

/// One array assignment observed in live code, e.g. `arr=()` or `arr+=(x)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArrayAssignment {
    pub name: String,
    /// True when the parenthesized initializer is empty (`arr=()`).
    pub empty: bool,
    /// Byte offset of the assignment within the line, for source ordering.
    pub offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Single,
    Double,
    Ansi,
    Comment,
    ParamExp,
    CmdSub,
    Arith,
    Backtick,
}

struct Scan {
    /// Same byte length as the input; blanked regions are spaces.
    masked: String,
    /// Byte offset of the `$` of each live `${...}` parameter expansion, paired
    /// with the offset just past its closing `}`.
    param_exps: Vec<Span>,
    /// Byte offset of the unquoted `#` that opens a trailing comment, if any.
    comment_start: Option<usize>,
    /// True when scanning ends inside an unterminated single-quoted string —
    /// the signal that a single-quoted awk program continues on the next line.
    open_single_quote: bool,
}

/// Walk a logical line once, producing its masked projection and the spans of
/// every live `${...}` parameter expansion.
fn scan(line: &str) -> Scan {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let n = chars.len();
    let end = line.len();
    let mut masked = String::with_capacity(end);
    let mut stack: Vec<Ctx> = Vec::new();
    let mut param_starts: Vec<usize> = Vec::new();
    let mut param_exps: Vec<Span> = Vec::new();
    let mut comment_start: Option<usize> = None;
    // A `#` opens a comment only at the start of a word in unquoted code.
    let mut word_boundary = true;

    let blank = |masked: &mut String, c: char| {
        for _ in 0..c.len_utf8() {
            masked.push(' ');
        }
    };

    let mut i = 0;
    while i < n {
        let (_, c) = chars[i];
        match stack.last().copied() {
            Some(Ctx::Comment) => {
                blank(&mut masked, c);
                i += 1;
                continue;
            }
            Some(Ctx::Single) => {
                blank(&mut masked, c);
                if c == '\'' {
                    stack.pop();
                }
                i += 1;
                continue;
            }
            Some(Ctx::Ansi) => {
                blank(&mut masked, c);
                if c == '\\' && i + 1 < n {
                    blank(&mut masked, chars[i + 1].1);
                    i += 2;
                    continue;
                }
                if c == '\'' {
                    stack.pop();
                }
                i += 1;
                continue;
            }
            _ => {}
        }

        let top = stack.last().copied();
        let blanking = matches!(top, Some(Ctx::Double));

        // Backslash escapes the next character in code and double quotes. Blank
        // both so an escaped metacharacter can never form a spurious operator.
        if c == '\\' {
            blank(&mut masked, c);
            if i + 1 < n {
                blank(&mut masked, chars[i + 1].1);
                i += 2;
            } else {
                i += 1;
            }
            word_boundary = false;
            continue;
        }

        // Dollar-introduced expansions and quoting forms.
        if c == '$' && i + 1 < n {
            let c1 = chars[i + 1].1;
            if c1 == '(' {
                if i + 2 < n && chars[i + 2].1 == '(' {
                    masked.push_str("$((");
                    stack.push(Ctx::Arith);
                    i += 3;
                } else {
                    masked.push_str("$(");
                    stack.push(Ctx::CmdSub);
                    i += 2;
                }
                word_boundary = false;
                continue;
            } else if c1 == '{' {
                masked.push_str("${");
                param_starts.push(chars[i].0);
                stack.push(Ctx::ParamExp);
                i += 2;
                word_boundary = false;
                continue;
            } else if c1 == '\'' {
                blank(&mut masked, '$');
                blank(&mut masked, '\'');
                stack.push(Ctx::Ansi);
                i += 2;
                word_boundary = false;
                continue;
            } else if c1 == '"' {
                blank(&mut masked, '$');
                blank(&mut masked, '"');
                stack.push(Ctx::Double);
                i += 2;
                word_boundary = false;
                continue;
            }
            // Simple `$name`, `$1`, `$@`, `$#`, ... — not a nesting form.
            if blanking {
                blank(&mut masked, c);
            } else {
                masked.push(c);
            }
            i += 1;
            word_boundary = false;
            continue;
        }

        if c == '\'' && !matches!(top, Some(Ctx::Double)) {
            blank(&mut masked, c);
            stack.push(Ctx::Single);
            i += 1;
            word_boundary = false;
            continue;
        }
        if c == '"' {
            blank(&mut masked, c);
            if matches!(top, Some(Ctx::Double)) {
                stack.pop();
            } else {
                stack.push(Ctx::Double);
            }
            i += 1;
            word_boundary = false;
            continue;
        }
        if c == '`' {
            masked.push('`');
            if matches!(top, Some(Ctx::Backtick)) {
                stack.pop();
            } else {
                stack.push(Ctx::Backtick);
            }
            i += 1;
            word_boundary = false;
            continue;
        }
        if c == '#' && top.is_none() && word_boundary {
            comment_start = Some(chars[i].0);
            blank(&mut masked, c);
            stack.push(Ctx::Comment);
            i += 1;
            continue;
        }
        if c == '}' && matches!(top, Some(Ctx::ParamExp)) {
            masked.push('}');
            stack.pop();
            if let Some(start) = param_starts.pop() {
                param_exps.push(Span {
                    start,
                    end: chars[i].0 + c.len_utf8(),
                });
            }
            i += 1;
            word_boundary = false;
            continue;
        }
        if c == ')' && matches!(top, Some(Ctx::CmdSub)) {
            masked.push(')');
            stack.pop();
            i += 1;
            word_boundary = false;
            continue;
        }
        if c == ')' && matches!(top, Some(Ctx::Arith)) {
            if i + 1 < n && chars[i + 1].1 == ')' {
                masked.push_str("))");
                stack.pop();
                i += 2;
            } else {
                masked.push(')');
                i += 1;
            }
            word_boundary = false;
            continue;
        }

        if blanking {
            blank(&mut masked, c);
        } else {
            masked.push(c);
        }
        word_boundary = c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(');
        i += 1;
    }

    debug_assert_eq!(masked.len(), end);
    let open_single_quote = matches!(stack.last(), Some(Ctx::Single));
    Scan {
        masked,
        param_exps,
        comment_start,
        open_single_quote,
    }
}

/// The trailing comment of a logical line (the `#` and everything after it),
/// or `None` when the line has no unquoted comment. A `#` inside a string is
/// never a comment.
pub(crate) fn line_comment(line: &str) -> Option<&str> {
    scan(line).comment_start.map(|start| &line[start..])
}

/// True when `line` ends inside an unterminated single-quoted string — i.e. a
/// single-quoted program (as in `awk 'BEGIN {`) continues on the next line. This
/// uses the lexer rather than counting raw quotes, so an apostrophe in a comment
/// or a `'\''` escape does not flip the result.
pub(crate) fn continues_single_quote(line: &str) -> bool {
    scan(line).open_single_quote
}

/// True when a reason-bearing `marker` waives the construct on `line`: either in
/// a genuine comment on `line` itself, or on `previous` when `previous` is a
/// standalone comment line.
///
/// The marker must sit inside an actual comment (so string or command text can
/// never waive a construct), and a *trailing* waiver stays construct-local —
/// only a full-line comment above can waive the following line, so an inline
/// waiver on one command never bleeds onto the next.
pub(crate) fn reasoned_comment_marker(line: &str, previous: &str, marker: &str) -> bool {
    if line_comment(line).is_some_and(|comment| marker_has_reason(comment, marker)) {
        return true;
    }
    previous.trim_start().starts_with('#')
        && line_comment(previous).is_some_and(|comment| marker_has_reason(comment, marker))
}

fn marker_has_reason(text: &str, marker: &str) -> bool {
    text.find(marker).is_some_and(|index| {
        let remainder = &text[index + marker.len()..];
        remainder.chars().next().is_some_and(char::is_whitespace)
            && !remainder.trim_matches([' ', '-', '>']).is_empty()
    })
}

// ── G009: unsafe pattern-substitution replacements ──────────────────────────

/// Spans of every live `${var/pat/replacement}` (or `//`) whose replacement can
/// introduce an unquoted `&` — i.e. it still contains a live expansion after
/// quoting and escaping have been accounted for.
///
/// Because the scanner blanks inert and quoted text, a replacement is hazardous
/// exactly when its masked form still contains a `$`-expansion or its raw form
/// still contains an unquoted legacy `` `cmd` `` command substitution: a bare
/// `$rep`, `${rep}`, `$(cmd)`, or `` `cmd` `` survives, while `"$rep"`,
/// `'$rep'`, `$'...'`, `` "`cmd`" ``, `` \` ``, `\&`, or a literal stay inert.
pub(crate) fn hazardous_replacements(line: &str) -> Vec<Span> {
    let scan = scan(line);
    let mut spans = Vec::new();
    for exp in &scan.param_exps {
        let body = &scan.masked[exp.start..exp.end];
        if let Some(offset) = substitution_replacement(body) {
            // Both slices end just before the closing `}` of the expansion.
            let masked_replacement = &body[offset..body.len() - 1];
            let raw_replacement = &line[exp.start + offset..exp.end - 1];
            if has_live_expansion(masked_replacement) || has_live_backtick(raw_replacement) {
                spans.push(*exp);
            }
        }
    }
    spans
}

/// True when a masked replacement string still contains a live `$`-expansion —
/// a `$` immediately followed by a name, brace, paren, or special parameter.
/// A bare trailing `$` (a literal dollar) is not an expansion.
fn has_live_expansion(replacement: &str) -> bool {
    let bytes = replacement.as_bytes();
    bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'$'
            && bytes
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_alphanumeric() || b"_{(@*!#?-".contains(next))
    })
}

/// True when a raw replacement string contains a live legacy backtick command
/// substitution — a backtick outside single/double quotes and not escaped. Its
/// result lands unquoted in replacement position exactly like `$(cmd)`, so it
/// can introduce an unquoted `&`; a quoted `` "`cmd`" `` substitution produces
/// a quoted result and stays safe.
fn has_live_backtick(replacement: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' if !in_single => {
                chars.next();
            }
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '`' if !in_single && !in_double => return true,
            _ => {}
        }
    }
    false
}

/// Given the masked body of a `${...}` expansion (including the braces), return
/// the byte offset within `body` where the replacement begins when it is a
/// pattern substitution `${p/pat/repl}` or `${p//pat/repl}` (also `/#` and `/%`
/// anchored forms). Non-substitution operators and replacement-less deletions
/// return `None`.
fn substitution_replacement(body: &str) -> Option<usize> {
    let inner = body.strip_prefix("${")?.strip_suffix('}')?;
    // First char after `${` cannot be an indirection/length sigil for a
    // substitution: `${!x}` and `${#x}` are never pattern substitutions.
    let first = inner.chars().next()?;
    if first == '!' || first == '#' {
        return None;
    }
    // Advance past the parameter name and any `[subscript]` to the operator.
    let mut depth = 0usize;
    let mut op_at = None;
    for (index, ch) in inner.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            '/' if depth == 0 => {
                op_at = Some(index);
                break;
            }
            // Any other parameter operator means this is not a substitution.
            ':' | '#' | '%' | '-' | '+' | '=' | '?' | '^' | ',' if depth == 0 => return None,
            _ => {}
        }
    }
    let op_at = op_at?;
    let after_op = &inner[op_at + 1..];
    // Skip the global `//` marker or an anchor (`/#`, `/%`) if present.
    let pattern_start = after_op
        .strip_prefix('/')
        .or_else(|| after_op.strip_prefix('#'))
        .or_else(|| after_op.strip_prefix('%'))
        .unwrap_or(after_op);
    let pattern_offset = inner.len() - pattern_start.len();
    // The replacement begins after the first top-level `/` separating pattern
    // from replacement. No separator means a deletion — no replacement, safe.
    // A `/` inside a backtick command substitution belongs to the pattern.
    let mut depth = 0usize;
    let mut in_backtick = false;
    for (index, ch) in pattern_start.char_indices() {
        match ch {
            '`' => in_backtick = !in_backtick,
            _ if in_backtick => {}
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth = depth.saturating_sub(1),
            '/' if depth == 0 => {
                // `+ 2` converts an `inner` offset back to a `body` offset
                // (the stripped `${` prefix).
                return Some(2 + pattern_offset + index + 1);
            }
            _ => {}
        }
    }
    None
}

// ── G010: Bash-3.2-incompatible constructs ──────────────────────────────────

static BASH4_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (
            r"(?:^|[\s;|&({])declare\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*A[A-Za-z]*(?:[\s;|&)]|$)",
            "declare -A associative arrays",
        ),
        (
            r"(?:^|[\s;|&({])typeset\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*A[A-Za-z]*(?:[\s;|&)]|$)",
            "typeset -A associative arrays",
        ),
        (
            r"(?:^|[\s;|&({])declare\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*g[A-Za-z]*(?:[\s;|&)]|$)",
            "declare -g global variable",
        ),
        (
            r"(?:^|[\s;|&({])(?:mapfile|readarray)(?:[\s;|&)]|$)",
            "mapfile/readarray",
        ),
        (
            r"\$\{[!A-Za-z_@*][A-Za-z0-9_]*(?:\^\^?|,,?)",
            "parameter case conversion",
        ),
        (
            r"(?:^|[\s;|&({])declare\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*n[A-Za-z]*(?:[\s;|&)]|$)",
            "declare -n nameref",
        ),
        (
            r"(?:^|[\s;|&({])local\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*n[A-Za-z]*(?:[\s;|&)]|$)",
            "local -n nameref",
        ),
        (
            r"(?:^|[\s;|&({])wait\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*n[A-Za-z]*(?:[\s;|&)]|$)",
            "wait -n",
        ),
        (
            r"(?:^|[\s;|&({])shopt\s+(?:-[A-Za-z]+\s+)*-[A-Za-z]*s[A-Za-z]*\s+(?:[A-Za-z_][A-Za-z0-9_]*\s+)*globstar(?:[\s;|&)]|$)",
            "shopt -s globstar",
        ),
        (
            r"(?:^|[\s;|&({])coproc(?:\s+[A-Za-z_][A-Za-z0-9_]*)?\s*\{",
            "coproc",
        ),
        (
            r"\$\{[!A-Za-z_@*][A-Za-z0-9_]*\[\s*-[0-9]",
            "negative array index",
        ),
        (
            r"\{(?:-?[0-9]+|[A-Za-z])\.\.(?:-?[0-9]+|[A-Za-z])\.\.-?[0-9]",
            "stepped brace expansion",
        ),
    ]
    .into_iter()
    .map(|(pattern, label)| (Regex::new(pattern).unwrap(), label))
    .collect()
});

// `if`/`elif` must be in command position (line start, after a separator, or
// after `then`/`do`) so a bare word like `echo if command x` is not matched.
static COMMAND_CONDITION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:^|[;&|(]|&&|\|\||\bthen\b|\bdo\b)\s*(?:if|elif)\s+(?:!\s+)?command\s+([^\s;&|()]+)",
    )
    .unwrap()
});

/// Bash-4-and-later constructs present in the live code of `line`. These are
/// syntax, builtins, or options unavailable in the macOS Bash 3.2 target.
pub(crate) fn bash4_constructs(line: &str) -> Vec<Construct> {
    let masked = scan(line).masked;
    let mut out = Vec::new();
    detect_operators(&masked, &mut out);
    for (pattern, label) in BASH4_PATTERNS.iter() {
        for m in pattern.find_iter(&masked) {
            out.push(Construct {
                label,
                span: Span {
                    start: m.start(),
                    end: m.end(),
                },
            });
        }
    }
    out.sort_by_key(|construct| construct.span.start);
    out
}

fn detect_operators(masked: &str, out: &mut Vec<Construct>) {
    let bytes = masked.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        match bytes[i] {
            b';' if i + 2 < n && bytes[i + 1] == b';' && bytes[i + 2] == b'&' => {
                out.push(Construct {
                    label: ";;& case fallthrough",
                    span: Span {
                        start: i,
                        end: i + 3,
                    },
                });
                i += 3;
            }
            b';' if i + 1 < n && bytes[i + 1] == b'&' => {
                out.push(Construct {
                    label: ";& case fallthrough",
                    span: Span {
                        start: i,
                        end: i + 2,
                    },
                });
                i += 2;
            }
            b'|' if i + 1 < n && bytes[i + 1] == b'&' => {
                out.push(Construct {
                    label: "|& pipe shorthand",
                    span: Span {
                        start: i,
                        end: i + 2,
                    },
                });
                i += 2;
            }
            b'&' if i + 2 < n && bytes[i + 1] == b'>' && bytes[i + 2] == b'>' => {
                out.push(Construct {
                    label: "&>> append-all redirection",
                    span: Span {
                        start: i,
                        end: i + 3,
                    },
                });
                i += 3;
            }
            _ => i += 1,
        }
    }
}

/// Spans of `if`/`elif` conditions of the form `[!] command <word>` where
/// `<word>` is a command name (not `-v`/`-V`/`-p`). Under `set -e`, Bash 3.2
/// aborts the whole script when such a condition's command fails, so the caller
/// reports these only when errexit is in effect.
pub(crate) fn command_conditions(line: &str) -> Vec<Span> {
    let masked = scan(line).masked;
    let mut spans = Vec::new();
    for caps in COMMAND_CONDITION.captures_iter(&masked) {
        let word = &caps[1];
        if matches!(word, "-v" | "-V" | "-p") {
            continue;
        }
        let m = caps.get(0).expect("group 0 always present");
        spans.push(Span {
            start: m.start(),
            end: m.end(),
        });
    }
    spans
}

static SET_COMMAND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[;&|(]|&&|\|\|)\s*set\b([^;&|]*)").unwrap());

/// Lexical enablement of `errexit`/`nounset` from a `set` command on `line`.
/// Only `set` in command position counts; `set` as an argument does not.
pub(crate) fn set_flags(line: &str) -> SetFlags {
    let masked = scan(line).masked;
    let mut flags = SetFlags::default();
    for caps in SET_COMMAND.captures_iter(&masked) {
        let args: Vec<&str> = caps[1].split_whitespace().collect();
        let mut index = 0;
        while index < args.len() {
            let arg = args[index];
            if arg == "-o" {
                match args.get(index + 1).copied() {
                    Some("errexit") => flags.errexit = true,
                    Some("nounset") => flags.nounset = true,
                    _ => {}
                }
                index += 2;
            } else if arg.starts_with('-') && !arg.starts_with("--") {
                if arg.contains('e') {
                    flags.errexit = true;
                }
                if arg.contains('u') {
                    flags.nounset = true;
                }
                index += 1;
            } else {
                index += 1;
            }
        }
    }
    flags
}

static ARRAY_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s;|&({])(?:declare\s+|local\s+|typeset\s+|readonly\s+)?([A-Za-z_][A-Za-z0-9_]*)\+?=\(([^)]*)\)").unwrap()
});
static ARRAY_AT_EXPANSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\$\{([A-Za-z_][A-Za-z0-9_]*)\[([@*])\]\}$").unwrap());
static CONTROL_FLOW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s;&|(){}])(?:if|then|elif|else|fi|for|while|until|do|done|case|esac|select)(?:[\s;&|(){}]|$)").unwrap()
});
static FUNCTION_OPEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s;&])(?:function\s+)?[A-Za-z_][A-Za-z0-9_]*\s*\(\s*\)").unwrap()
});
// A command group `{ ...` or subshell `( ...` at command position — but not the
// `{` of a `${...}` expansion (which is always preceded by `$`).
static GROUP_OR_SUBSHELL_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s;&|])[({](?:\s|$)").unwrap());
// A closing `}` or `)` in command position ends a function body, group, or
// subshell. Requiring line start or a real separator before the closer keeps
// the `)` of `arr=( )` initializers and of mid-line `$( ... )` substitutions
// from being read as a scope exit.
static GROUP_OR_SUBSHELL_CLOSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[;&|])\s*[)}](?:[\s;&|]|$)").unwrap());

/// Array assignments in the live code of `line`, in source order. Emptiness is
/// judged from the raw initializer text, not the masked projection, so a
/// quoted-literal element (`arr=("x")`) is correctly seen as non-empty even
/// though the scanner blanks its quoted content.
pub(crate) fn array_assignments(line: &str) -> Vec<ArrayAssignment> {
    let masked = scan(line).masked;
    ARRAY_ASSIGNMENT
        .captures_iter(&masked)
        .map(|caps| {
            let values = caps.get(2).expect("group 2 always present");
            ArrayAssignment {
                name: caps[1].to_string(),
                empty: line[values.start()..values.end()].trim().is_empty(),
                offset: caps.get(1).expect("group 1 always present").start(),
            }
        })
        .collect()
}

/// Bare `${name[@]}` / `${name[*]}` expansions in the live code of `line`, each
/// as `(name, span)`. Only top-level expansions count: a `:-`/`+` default makes
/// an empty array safe, and an expansion nested inside another `${...}` (as in
/// the `${arr[@]+"${arr[@]}"}` guard idiom) is covered by its guarding parent.
pub(crate) fn unguarded_array_expansions(line: &str) -> Vec<(String, Span)> {
    let scan = scan(line);
    let mut expansions = Vec::new();
    for exp in &scan.param_exps {
        let nested = scan
            .param_exps
            .iter()
            .any(|other| other != exp && other.start <= exp.start && exp.end <= other.end);
        if nested {
            continue;
        }
        let body = &scan.masked[exp.start..exp.end];
        if let Some(caps) = ARRAY_AT_EXPANSION.captures(body) {
            expansions.push((caps[1].to_string(), *exp));
        }
    }
    expansions
}

/// True when the live code of `line` crosses a control-flow or scope boundary:
/// a conditional, loop, function entry or exit, command group or subshell open
/// or close, case-arm terminator (`;;`/`;&`/`;;&`), or `&&`/`||` short-circuit.
/// The empty-array analysis treats any such boundary as making array contents
/// ambiguous, so facts never leak across function, group, or branch scopes in
/// either direction.
pub(crate) fn control_flow_boundary(line: &str) -> bool {
    let masked = scan(line).masked;
    masked.contains("&&")
        || masked.contains("||")
        || masked.contains(";;")
        || masked.contains(";&")
        || CONTROL_FLOW.is_match(&masked)
        || FUNCTION_OPEN.is_match(&masked)
        || GROUP_OR_SUBSHELL_OPEN.is_match(&masked)
        || GROUP_OR_SUBSHELL_CLOSE.is_match(&masked)
}

// ── G011: non-ASCII awk regex operands ──────────────────────────────────────

/// A parsed `awk` command line: its option-supplied regex operands plus the
/// variable assignments whose regex use must be traced through the program.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AwkCommand {
    /// Field separator value (`-F`, `--field-separator`), if any.
    pub field_separator: Option<String>,
    /// `-v name=value` assignments, in order.
    pub assignments: Vec<(String, String)>,
}

/// Findings from analyzing one complete awk invocation.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AwkAnalysis {
    /// Non-ASCII regex operands supplied on the command line (`-F`, `-v FS=`, or
    /// a `-v` value later used as a regex). Reported at the command line.
    pub option_evidence: Vec<String>,
    /// Non-ASCII regex operands in the program body, as `(line_offset, evidence)`
    /// where `line_offset` counts newlines before the operand within `program`.
    pub program_findings: Vec<(usize, String)>,
}

/// Parse the options of an `awk ...` shell command. Returns `None` when the line
/// contains no awk invocation. The inline program operand is intentionally not
/// returned; the caller supplies the full (possibly multi-line) program text.
pub(crate) fn parse_awk_command(line: &str) -> Option<AwkCommand> {
    let tokens: Vec<String> = unquoted_tokens(line)
        .into_iter()
        .map(|token| token.text)
        .collect();
    let awk_at = tokens
        .iter()
        .position(|token| token == "awk" || token.ends_with("/awk"))?;
    let mut command = AwkCommand::default();
    let mut index = awk_at + 1;
    while index < tokens.len() {
        let token = &tokens[index];
        if matches!(token.as_str(), "|" | ";" | "&" | "&&" | "||" | "|&") {
            break;
        }
        if token == "--" {
            break;
        } else if token == "-F" || token == "--field-separator" {
            if let Some(value) = tokens.get(index + 1) {
                command.field_separator = Some(value.clone());
            }
            index += 2;
        } else if let Some(value) = token
            .strip_prefix("--field-separator=")
            .or_else(|| token.strip_prefix("-F"))
        {
            command.field_separator = Some(value.to_string());
            index += 1;
        } else if token == "-v" {
            if let Some(assignment) = tokens.get(index + 1) {
                push_assignment(&mut command, assignment);
            }
            index += 2;
        } else if let Some(assignment) = token.strip_prefix("-v") {
            push_assignment(&mut command, assignment);
            index += 1;
        } else if token == "-f" {
            index += 2;
        } else if token.starts_with('-') && token.len() > 1 {
            index += 1;
        } else {
            // The first operand is the program (or an input file); options end.
            break;
        }
    }
    Some(command)
}

/// Extract the first inline awk program operand from `text` (which may span
/// multiple joined lines), returning its unquoted content and the byte offset
/// of that content within `text`. Returns `None` when awk reads its program
/// from `-f` (a file or stdin) and there is no inline operand.
pub(crate) fn inline_awk_program(text: &str) -> Option<(String, usize)> {
    let tokens = unquoted_tokens(text);
    let awk_at = tokens
        .iter()
        .position(|token| token.text == "awk" || token.text.ends_with("/awk"))?;
    let mut index = awk_at + 1;
    while let Some(token) = tokens.get(index) {
        let value = token.text.as_str();
        if matches!(value, "|" | ";" | "&" | "&&" | "||" | "|&") {
            return None;
        }
        if value == "--" {
            index += 1;
            break;
        } else if value == "-F" || value == "--field-separator" || value == "-v" {
            index += 2;
        } else if value == "-f" {
            // Program comes from a file/stdin, not an inline operand.
            return None;
        } else if value.starts_with('-') && value.len() > 1 {
            index += 1;
        } else {
            break;
        }
    }
    let token = tokens.get(index)?;
    Some((token.text.clone(), token.content_start))
}

/// True when an awk command reads its program from standard input — `-f -` or
/// `-f /dev/stdin` — so an accompanying heredoc is the program, not input data.
/// A `-f file` reference (program from a named file) returns false.
pub(crate) fn awk_program_from_stdin(line: &str) -> bool {
    let tokens = unquoted_tokens(line);
    let Some(awk_at) = tokens
        .iter()
        .position(|token| token.text == "awk" || token.text.ends_with("/awk"))
    else {
        return false;
    };
    let mut index = awk_at + 1;
    while let Some(token) = tokens.get(index) {
        let value = token.text.as_str();
        if matches!(value, "|" | ";" | "&" | "&&" | "||" | "|&" | "--") {
            return false;
        }
        if value == "-f" {
            return matches!(
                tokens.get(index + 1).map(|t| t.text.as_str()),
                Some("-" | "/dev/stdin")
            );
        } else if let Some(rest) = value.strip_prefix("-f") {
            return matches!(rest, "-" | "/dev/stdin");
        } else if value == "-F" || value == "--field-separator" || value == "-v" {
            index += 2;
        } else if value.starts_with('-') && value.len() > 1 {
            index += 1;
        } else {
            return false;
        }
    }
    false
}

fn push_assignment(command: &mut AwkCommand, assignment: &str) {
    if let Some((name, value)) = assignment.split_once('=') {
        command
            .assignments
            .push((name.to_string(), value.to_string()));
    }
}

/// Analyze one complete awk invocation. `command_line` supplies the options (it
/// may be empty for a standalone `.awk` file whose separators come from an
/// unknown caller); `program` is the full program text.
pub(crate) fn analyze_awk(command_line: &str, program: &str) -> AwkAnalysis {
    let mut analysis = AwkAnalysis::default();
    let regex_vars = awk_regex_variables(program);

    if !command_line.is_empty() {
        if let Some(command) = parse_awk_command(command_line) {
            if let Some(separator) = &command.field_separator {
                if !separator.is_ascii() {
                    analysis.option_evidence.push(separator.clone());
                }
            }
            for (name, value) in &command.assignments {
                if (name == "FS" || regex_vars.contains(name)) && !value.is_ascii() {
                    analysis.option_evidence.push(value.clone());
                }
            }
        }
    }

    for (offset, evidence) in awk_literal_regex_operands(program) {
        if !evidence.is_ascii() {
            let line_offset = program[..offset].bytes().filter(|b| *b == b'\n').count();
            analysis.program_findings.push((line_offset, evidence));
        }
    }
    for (offset, evidence) in awk_definite_constant_regex_flows(program) {
        if !evidence.is_ascii() {
            let line_offset = program[..offset].bytes().filter(|b| *b == b'\n').count();
            analysis.program_findings.push((line_offset, evidence));
        }
    }
    analysis.program_findings.sort_by_key(|(line, _)| *line);
    analysis
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AwkTok {
    Ident(String),
    Str(String),
    Regex(String),
    Punct(String),
    Other,
}

struct AwkToken {
    tok: AwkTok,
    offset: usize,
}

/// Tokenize an awk program, distinguishing regex literals from division and
/// tracking string contents. Comments and whitespace are dropped.
fn awk_tokens(program: &str) -> Vec<AwkToken> {
    let chars: Vec<(usize, char)> = program.char_indices().collect();
    let n = chars.len();
    let mut tokens: Vec<AwkToken> = Vec::new();
    let mut i = 0;
    // A `/` opens a regex when the previous significant token is not a value.
    let mut expect_regex = true;
    while i < n {
        let (offset, c) = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '#' {
            while i < n && chars[i].1 != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '"' {
            let mut content = String::new();
            i += 1;
            while i < n {
                let ch = chars[i].1;
                if ch == '\\' && i + 1 < n {
                    content.push(chars[i + 1].1);
                    i += 2;
                    continue;
                }
                if ch == '"' {
                    i += 1;
                    break;
                }
                content.push(ch);
                i += 1;
            }
            tokens.push(AwkToken {
                tok: AwkTok::Str(content),
                offset,
            });
            expect_regex = false;
            continue;
        }
        if c == '/' && expect_regex {
            let mut content = String::new();
            let mut in_class = false;
            i += 1;
            while i < n {
                let ch = chars[i].1;
                if ch == '\\' && i + 1 < n {
                    content.push(ch);
                    content.push(chars[i + 1].1);
                    i += 2;
                    continue;
                }
                if ch == '[' {
                    in_class = true;
                } else if ch == ']' {
                    in_class = false;
                } else if ch == '/' && !in_class {
                    i += 1;
                    break;
                } else if ch == '\n' {
                    break;
                }
                content.push(ch);
                i += 1;
            }
            tokens.push(AwkToken {
                tok: AwkTok::Regex(content),
                offset,
            });
            expect_regex = false;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < n && (chars[i].1.is_alphanumeric() || chars[i].1 == '_') {
                i += 1;
            }
            let text: String = chars[start..i].iter().map(|(_, ch)| *ch).collect();
            tokens.push(AwkToken {
                tok: AwkTok::Ident(text),
                offset,
            });
            expect_regex = false;
            continue;
        }
        if c.is_ascii_digit() {
            while i < n && (chars[i].1.is_alphanumeric() || chars[i].1 == '.') {
                i += 1;
            }
            tokens.push(AwkToken {
                tok: AwkTok::Other,
                offset,
            });
            expect_regex = false;
            continue;
        }
        // Punctuation and operators.
        if c == '!' && i + 1 < n && chars[i + 1].1 == '~' {
            tokens.push(AwkToken {
                tok: AwkTok::Punct("!~".to_string()),
                offset,
            });
            i += 2;
            expect_regex = true;
            continue;
        }
        let punct = c.to_string();
        // After a value-producing closer, `/` is division; after everything
        // else it opens a regex.
        expect_regex = !matches!(c, ')' | ']');
        tokens.push(AwkToken {
            tok: AwkTok::Punct(punct),
            offset,
        });
        i += 1;
    }
    tokens
}

/// Variable names used as a regex operand somewhere in the program.
fn awk_regex_variables(program: &str) -> HashSet<String> {
    let tokens = awk_tokens(program);
    let mut vars = HashSet::new();
    collect_regex_operands(&tokens, &mut |operand| {
        if let AwkTok::Ident(name) = operand {
            vars.insert(name.clone());
        }
    });
    vars
}

/// Literal (string or `/.../`) regex operands in the program, each as
/// `(byte_offset, text)`.
fn awk_literal_regex_operands(program: &str) -> Vec<(usize, String)> {
    let tokens = awk_tokens(program);
    let mut operands = Vec::new();
    // Every `/.../` literal is always an ERE, wherever it appears.
    for token in &tokens {
        if let AwkTok::Regex(content) = &token.tok {
            operands.push((token.offset, content.clone()));
        }
    }
    // String literals count only when they sit in a regex-operand position.
    collect_regex_operands_with_offset(&tokens, &mut |offset, operand| {
        if let AwkTok::Str(content) = operand {
            operands.push((offset, content.clone()));
        }
    });
    operands.sort_by_key(|(offset, _)| *offset);
    operands.dedup();
    operands
}

/// Regex operands reached through definite in-program constant flow: a simple
/// string assignment `name = "value"` that is the variable's only modification
/// in the program, sits outside every conditional construct, and textually
/// precedes a use of the variable in a regex-operand position. Each result is
/// `(byte_offset_of_the_literal, value)`; reassignment ambiguity, computed or
/// branch-dependent values, and caller-supplied variables never qualify.
fn awk_definite_constant_regex_flows(program: &str) -> Vec<(usize, String)> {
    let tokens = awk_tokens(program);
    // Offsets of every variable used in a regex-operand position, per name.
    let mut uses: Vec<(usize, String)> = Vec::new();
    collect_regex_operands_with_offset(&tokens, &mut |offset, operand| {
        if let AwkTok::Ident(name) = operand {
            uses.push((offset, name.clone()));
        }
    });
    let counts = assignment_like_counts(&tokens);
    simple_unconditional_assignments(program, &tokens)
        .into_iter()
        .filter(|assignment| {
            // `FS = "..."` is already reported as a direct operand; the flow
            // path covers every other variable exactly once.
            assignment.name != "FS"
                && counts.get(&assignment.name).copied().unwrap_or(0) == 1
                && uses
                    .iter()
                    .any(|(offset, name)| *name == assignment.name && *offset > assignment.offset)
        })
        .map(|assignment| (assignment.offset, assignment.value))
        .collect()
}

/// One `name = "value"` statement whose RHS is a single string literal.
struct SimpleAssignment {
    name: String,
    value: String,
    /// Byte offset of the string literal (the reported evidence).
    offset: usize,
}

/// True for tokens that complete a value, so a newline after them terminates
/// the statement (an operator would instead continue it onto the next line).
fn value_ender(token: &AwkTok) -> bool {
    match token {
        AwkTok::Ident(_) | AwkTok::Str(_) | AwkTok::Regex(_) | AwkTok::Other => true,
        AwkTok::Punct(op) => matches!(op.as_str(), ")" | "]"),
    }
}

/// Simple string assignments located outside every conditional construct. The
/// walk conservatively marks everything below `if`/`else`/`while`/`for`/`do`/
/// `function` bodies and `?:` arms as conditional — brace-delimited or not —
/// and at the top level only `BEGIN`/`END` blocks are unconditional: a
/// pattern-guarded or bare main-rule action runs per matching input line, so
/// its assignments stay ambiguous. Ambiguity always errs toward returning
/// nothing.
fn simple_unconditional_assignments(program: &str, tokens: &[AwkToken]) -> Vec<SimpleAssignment> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Pending {
        /// On the unconditional path of the current block.
        None,
        /// After `if`/`while`/`for`/`function`, waiting for its parenthesized
        /// header (opened at `depth`) to close.
        Condition { depth: usize },
        /// A conditional body is next (header closed, or after `else`/`do`/`?`).
        Body,
        /// Inside a brace-less conditional body; ends at a statement-level `;`,
        /// a `}`, or a statement-terminating newline.
        BracelessBody,
    }
    let mut out = Vec::new();
    let mut pending = Pending::None;
    let mut paren_depth = 0usize;
    // One entry per open `{`: true when the block is unconditional.
    let mut safe_stack: Vec<bool> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        // A statement-terminating newline ends a brace-less conditional body:
        // the previous token must complete a value, so a trailing operator
        // keeps the body open across the line break.
        if pending == Pending::BracelessBody
            && paren_depth == 0
            && index > 0
            && program[tokens[index - 1].offset..token.offset].contains('\n')
            && value_ender(&tokens[index - 1].tok)
        {
            pending = Pending::None;
        }
        match &token.tok {
            AwkTok::Ident(name) if matches!(name.as_str(), "if" | "while" | "for" | "function") => {
                pending = Pending::Condition { depth: paren_depth };
            }
            AwkTok::Ident(name) if matches!(name.as_str(), "else" | "do") => {
                pending = Pending::Body;
            }
            AwkTok::Punct(op) => match op.as_str() {
                "(" => {
                    paren_depth += 1;
                    if pending == Pending::Body {
                        pending = Pending::BracelessBody;
                    }
                }
                ")" => {
                    paren_depth = paren_depth.saturating_sub(1);
                    if pending == (Pending::Condition { depth: paren_depth }) {
                        pending = Pending::Body;
                    }
                }
                "{" => {
                    // A nested `{` is a plain group when nothing conditional is
                    // pending; a top-level `{` is unconditional only for the
                    // BEGIN and END blocks (a pattern action or bare main rule
                    // runs per matching input line).
                    let unconditional_rule = safe_stack.is_empty()
                        && index > 0
                        && matches!(
                            &tokens[index - 1].tok,
                            AwkTok::Ident(name) if name == "BEGIN" || name == "END"
                        );
                    let safe =
                        pending == Pending::None && (unconditional_rule || !safe_stack.is_empty());
                    safe_stack.push(safe);
                    pending = Pending::None;
                }
                "}" => {
                    safe_stack.pop();
                    pending = Pending::None;
                }
                ";" if paren_depth == 0 => pending = Pending::None,
                "?" => pending = Pending::Body,
                _ => {
                    if pending == Pending::Body {
                        pending = Pending::BracelessBody;
                    }
                }
            },
            _ => {
                if pending == Pending::Body {
                    pending = Pending::BracelessBody;
                }
            }
        }
        let (AwkTok::Ident(name), Some(eq), Some(rhs)) =
            (&token.tok, tokens.get(index + 1), tokens.get(index + 2))
        else {
            continue;
        };
        if !matches!(&eq.tok, AwkTok::Punct(op) if op == "=") {
            continue;
        }
        let AwkTok::Str(value) = &rhs.tok else {
            continue;
        };
        // The RHS is exactly one literal only when the next token cannot
        // continue the expression: a terminator on the same line, the end of
        // the program, or a statement-separating newline. Concatenation
        // (`"a" tail`) and continuation operators disqualify the assignment.
        let simple_end = match tokens.get(index + 3) {
            None => true,
            Some(next) => {
                matches!(&next.tok, AwkTok::Punct(op) if matches!(op.as_str(), ";" | "}" | ")" | ","))
                    || program[rhs.offset..next.offset].contains('\n')
            }
        };
        if simple_end && pending == Pending::None && safe_stack.iter().all(|safe| *safe) {
            out.push(SimpleAssignment {
                name: name.clone(),
                value: value.clone(),
                offset: rhs.offset,
            });
        }
    }
    out
}

/// How many times each variable is assigned or modified anywhere in the
/// program: plain and compound assignments, increments/decrements, `getline`
/// targets, and the in-place target of `sub`/`gsub`. Comparison operators are
/// not modifications. Used to reject reassignment ambiguity.
fn assignment_like_counts(tokens: &[AwkToken]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let punct = |token: Option<&AwkToken>, expected: &str| matches!(token.map(|t| &t.tok), Some(AwkTok::Punct(op)) if op == expected);
    for (index, token) in tokens.iter().enumerate() {
        match &token.tok {
            AwkTok::Ident(name) if name == "getline" => {
                // `getline var` (also `cmd | getline var`) assigns the target.
                if let Some(AwkTok::Ident(target)) = tokens.get(index + 1).map(|t| &t.tok) {
                    *counts.entry(target.clone()).or_insert(0) += 1;
                }
            }
            AwkTok::Ident(name) if matches!(name.as_str(), "sub" | "gsub") => {
                // The optional third argument is modified in place.
                if punct(tokens.get(index + 1), "(") {
                    if let Some(target) = call_argument(tokens, index + 1, 2) {
                        if let AwkTok::Ident(target) = &tokens[target].tok {
                            *counts.entry(target.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
            AwkTok::Ident(name) => {
                let assigned = if punct(tokens.get(index + 1), "=") {
                    // `name = ...` but not the `==` comparison.
                    !punct(tokens.get(index + 2), "=")
                } else {
                    // Compound assignment (`+=` ... `^=`) or `++`/`--`.
                    ["+", "-", "*", "/", "%", "^"].iter().any(|op| {
                        punct(tokens.get(index + 1), op)
                            && (punct(tokens.get(index + 2), "=")
                                || (matches!(*op, "+" | "-") && punct(tokens.get(index + 2), op)))
                    })
                };
                if assigned {
                    *counts.entry(name.clone()).or_insert(0) += 1;
                }
            }
            AwkTok::Punct(op) if matches!(op.as_str(), "+" | "-") => {
                // Prefix increment/decrement: `++name` / `--name`.
                if punct(tokens.get(index + 1), op) {
                    if let Some(AwkTok::Ident(target)) = tokens.get(index + 2).map(|t| &t.tok) {
                        *counts.entry(target.clone()).or_insert(0) += 1;
                    }
                }
            }
            _ => {}
        }
    }
    counts
}

fn collect_regex_operands(tokens: &[AwkToken], visit: &mut impl FnMut(&AwkTok)) {
    collect_regex_operands_with_offset(tokens, &mut |_, operand| visit(operand));
}

/// Walk the token stream and hand each regex-operand token (and its offset) to
/// `visit`: the RHS of `~`/`!~`, the regex argument of `match`/`sub`/`gsub`/
/// `gensub`/`split`/`patsplit`, and the RHS of an `FS = ...` assignment.
fn collect_regex_operands_with_offset(tokens: &[AwkToken], visit: &mut impl FnMut(usize, &AwkTok)) {
    for (index, token) in tokens.iter().enumerate() {
        match &token.tok {
            AwkTok::Punct(op) if op == "~" || op == "!~" => {
                if let Some(next) = tokens.get(index + 1) {
                    visit(next.offset, &next.tok);
                }
            }
            AwkTok::Ident(name) if name == "FS" => {
                if let (Some(eq), Some(rhs)) = (tokens.get(index + 1), tokens.get(index + 2)) {
                    if matches!(&eq.tok, AwkTok::Punct(op) if op == "=") {
                        visit(rhs.offset, &rhs.tok);
                    }
                }
            }
            AwkTok::Ident(name) => {
                let arg = match name.as_str() {
                    "sub" | "gsub" | "gensub" => Some(0),
                    "match" => Some(1),
                    "split" | "patsplit" => Some(2),
                    _ => None,
                };
                if let Some(arg_index) = arg {
                    if let Some(next) = tokens.get(index + 1) {
                        if matches!(&next.tok, AwkTok::Punct(op) if op == "(") {
                            if let Some(operand) = call_argument(tokens, index + 1, arg_index) {
                                let token = &tokens[operand];
                                visit(token.offset, &token.tok);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Index of the single token forming argument `arg_index` (0-based) of a call
/// whose `(` is at `open`, or `None` when that argument is absent or is a
/// compound expression rather than one literal/identifier.
fn call_argument(tokens: &[AwkToken], open: usize, arg_index: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut current = 0usize;
    let mut start = open + 1;
    let mut count = 0usize;
    let mut end = None;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        if let AwkTok::Punct(op) = &token.tok {
            match op.as_str() {
                "(" | "[" => depth += 1,
                ")" | "]" => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(index);
                        break;
                    }
                }
                "," if depth == 1 => {
                    if current == arg_index {
                        end = Some(index);
                        break;
                    }
                    current += 1;
                    start = index + 1;
                    count = 0;
                    continue;
                }
                _ => {}
            }
        }
        if index > open {
            count += 1;
        }
    }
    let end = end?;
    if current != arg_index {
        return None;
    }
    // Exactly one token between `start` and `end` makes an atomic operand.
    let _ = count;
    if end == start + 1 { Some(start) } else { None }
}

/// A shell word or operator with the unquoted text plus source offsets: the
/// token start and the offset of its first content character (past an opening
/// quote for a quoted word).
struct SpannedToken {
    text: String,
    content_start: usize,
}

/// Tokenize a shell command, unquoting words the way the shell would, so option
/// values are available verbatim. Operators become their own tokens. Offsets are
/// byte positions within `command`.
fn unquoted_tokens(command: &str) -> Vec<SpannedToken> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut has_word = false;
    let mut content_start = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut chars = command.char_indices().peekable();
    while let Some((offset, c)) = chars.next() {
        if escaped {
            if !has_word {
                content_start = offset;
                has_word = true;
            }
            current.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            if !has_word {
                content_start = offset;
                has_word = true;
            }
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if c == active {
                quote = None;
            } else {
                current.push(c);
            }
            continue;
        }
        if c == '\'' || c == '"' {
            if !has_word {
                content_start = offset + c.len_utf8();
                has_word = true;
            }
            quote = Some(c);
        } else if c.is_whitespace() {
            if has_word {
                tokens.push(SpannedToken {
                    text: std::mem::take(&mut current),
                    content_start,
                });
                has_word = false;
            }
        } else if "|;&".contains(c) {
            if has_word {
                tokens.push(SpannedToken {
                    text: std::mem::take(&mut current),
                    content_start,
                });
                has_word = false;
            }
            let mut operator = c.to_string();
            if chars
                .peek()
                .is_some_and(|(_, next)| (*next == c && c != ';') || (c == '|' && *next == '&'))
            {
                operator.push(chars.next().map(|(_, ch)| ch).unwrap_or_default());
            }
            tokens.push(SpannedToken {
                text: operator,
                content_start: offset,
            });
        } else {
            if !has_word {
                content_start = offset;
                has_word = true;
            }
            current.push(c);
        }
    }
    if has_word {
        tokens.push(SpannedToken {
            text: current,
            content_start,
        });
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(line: &str) -> String {
        scan(line).masked
    }

    #[test]
    fn masking_preserves_byte_offsets_and_blanks_inert_text() {
        let line = "echo 'テスト' # コメント";
        let m = masked(line);
        assert_eq!(m.len(), line.len());
        assert!(m.starts_with("echo "));
        assert!(!m.contains('テ'));
        assert!(!m.contains('#'));
    }

    #[test]
    fn masking_keeps_expansions_live_inside_double_quotes() {
        let m = masked(r#"printf '%s' "${arr[@]}" plain"#);
        assert!(m.contains("${arr[@]}"), "masked: {m:?}");
        assert!(!m.contains("plain") || m.contains("plain"));
    }

    #[test]
    fn g009_flags_dynamic_replacements_and_spares_quoted_ones() {
        assert_eq!(
            hazardous_replacements(r#"out="${out//TOKEN/$rep}""#).len(),
            1
        );
        assert_eq!(
            hazardous_replacements(r#"out="${out//TOKEN/${rep}}""#).len(),
            1
        );
        assert_eq!(hazardous_replacements("out=${text/TOKEN/$rep}").len(), 1);
        assert_eq!(
            hazardous_replacements(r#"out="${out//TOKEN/$(printf x)}""#).len(),
            1
        );
        // Quoted, ANSI-C, escaped, or literal replacements are safe.
        assert!(hazardous_replacements(r#"out="${out//TOKEN/"$rep"}""#).is_empty());
        assert!(hazardous_replacements(r#"out="${out//$'\n'/$'\n  '}""#).is_empty());
        assert!(hazardous_replacements(r"out=${text//TOKEN/\&}").is_empty());
        assert!(hazardous_replacements("out=${text//TOKEN/literal}").is_empty());
        // Prefix/suffix removal is not a substitution.
        assert!(hazardous_replacements(r#"x="${body%%TOKEN*}""#).is_empty());
        assert!(hazardous_replacements(r#"x="${body##*TOKEN}""#).is_empty());
        // Single-quoted or commented lines do not fire.
        assert!(hazardous_replacements(r"printf '%s' '${out//TOKEN/$rep}'").is_empty());
        assert!(hazardous_replacements("true # out=${out//TOKEN/$rep}").is_empty());
        // A lone literal `$` in the replacement is not a live expansion.
        assert!(hazardous_replacements("out=${text//TOKEN/$}").is_empty());
        // Positional and special parameters are live expansions.
        assert_eq!(hazardous_replacements("out=${text//TOKEN/$1}").len(), 1);
        assert_eq!(
            hazardous_replacements("out=${text//TOKEN/${arr[@]}}").len(),
            1
        );
    }

    #[test]
    fn g009_flags_live_backtick_replacements_and_spares_quoted_ones() {
        // Legacy backtick command substitution in replacement position is
        // dynamic exactly like `$(...)` (#550), in both `/` and `//` forms.
        assert_eq!(
            hazardous_replacements(r#"out=${text/TOKEN/`printf '%s' "$replacement"`}"#).len(),
            1
        );
        assert_eq!(
            hazardous_replacements(r#"out=${text//TOKEN/`printf '%s' "$replacement"`}"#).len(),
            1
        );
        assert_eq!(hazardous_replacements("out=${text//TOKEN/`cmd`}").len(), 1);
        // A double-quoted substitution produces a quoted result; a
        // single-quoted or escaped backtick is literal text.
        assert!(hazardous_replacements(r#"out=${text//TOKEN/"`cmd`"}"#).is_empty());
        assert!(hazardous_replacements("out=${text//TOKEN/'`cmd`'}").is_empty());
        assert!(hazardous_replacements(r"out=${text//TOKEN/\`}").is_empty());
        // Inert contexts never fire: single quotes and comments.
        assert!(hazardous_replacements("printf '%s' '${text/TOKEN/`cmd`}'").is_empty());
        assert!(hazardous_replacements("true # out=${text/TOKEN/`cmd`}").is_empty());
        // A backtick in the pattern (not the replacement) is not a hazard, and
        // a `/` inside it does not shift the replacement boundary.
        assert!(hazardous_replacements("out=${text/`cat a/b`/x}").is_empty());
    }

    #[test]
    fn g010_reports_bash4_syntax_and_ignores_inert_text() {
        let labels = |line: &str| -> Vec<&'static str> {
            bash4_constructs(line)
                .into_iter()
                .map(|c| c.label)
                .collect()
        };
        assert!(labels("declare -A m").contains(&"declare -A associative arrays"));
        assert!(labels("declare -g X=1").contains(&"declare -g global variable"));
        assert!(labels("wait -n").contains(&"wait -n"));
        assert!(labels("shopt -s globstar").contains(&"shopt -s globstar"));
        assert!(labels("case x in x) : ;& esac").contains(&";& case fallthrough"));
        assert!(labels("case x in x) : ;;& esac").contains(&";;& case fallthrough"));
        assert!(labels("echo hi |& cat").contains(&"|& pipe shorthand"));
        assert!(labels("printf '%s' \"${NAME^^}\"").contains(&"parameter case conversion"));
        // Inert contexts stay clean.
        assert!(bash4_constructs("true # declare -A documented=()").is_empty());
        assert!(bash4_constructs("echo 'declare -A literal'").is_empty());
        // Supported Bash 3.2 syntax is not flagged.
        assert!(bash4_constructs("echo a;; b 2>/dev/null || true").is_empty());
        assert!(bash4_constructs("cmd1 && cmd2 || cmd3").is_empty());
    }

    #[test]
    fn g010_command_condition_respects_the_v_exemption() {
        assert_eq!(
            command_conditions("if command grep -q x f; then :; fi").len(),
            1
        );
        assert_eq!(
            command_conditions("elif command sed -n q f; then :; fi").len(),
            1
        );
        assert_eq!(
            command_conditions("if ! command false; then :; fi").len(),
            1
        );
        assert!(command_conditions("if command -v tool >/dev/null; then :; fi").is_empty());
        assert!(command_conditions("if ( command grep -q x f ); then :; fi").is_empty());
        // `if`/`elif` must be in command position, not an argument word.
        assert!(command_conditions("echo if command grep is fine").is_empty());
        assert!(command_conditions("printf '%s' elif command x").is_empty());
        // After a separator or `then`/`do` it is a real condition.
        assert_eq!(
            command_conditions("foo; if command grep -q x f; then :; fi").len(),
            1
        );
    }

    #[test]
    fn array_emptiness_reads_raw_initializer_not_masked_text() {
        // A quoted-literal element is non-empty even though the scanner blanks
        // its content (reviewer finding #1).
        assert!(!array_assignments(r#"arr=("literal")"#)[0].empty);
        assert!(!array_assignments(r#"args=("$1" "$2")"#)[0].empty);
        assert!(array_assignments("arr=()")[0].empty);
        assert!(array_assignments("arr=(  )")[0].empty);
        assert!(!array_assignments("arr=(a b)")[0].empty);
    }

    #[test]
    fn single_quote_continuation_uses_the_lexer_not_raw_counts() {
        // A complete program with an apostrophe in a trailing comment does not
        // look like it continues (reviewer finding #2).
        assert!(!continues_single_quote("awk '{ print }'  # don't touch"));
        assert!(!continues_single_quote(
            r#"awk 'BEGIN { print "it'\''s" }'"#
        ));
        // An unterminated single-quoted program does continue.
        assert!(continues_single_quote("awk 'BEGIN {"));
        assert!(!continues_single_quote("awk 'BEGIN {\n  print\n}'"));
    }

    #[test]
    fn line_comment_ignores_hashes_inside_strings() {
        // A `#` in a string is not a comment (reviewer finding #3).
        assert_eq!(line_comment("declare -A m; msg='# not a comment'"), None);
        assert_eq!(line_comment(r#"echo "a # b""#), None);
        assert_eq!(
            line_comment("declare -A m # real comment"),
            Some("# real comment")
        );
    }

    #[test]
    fn awk_program_source_distinguishes_stdin_from_files() {
        // Only `-f -`/stdin means the heredoc is the program (reviewer finding #5).
        assert!(awk_program_from_stdin("awk -f - "));
        assert!(awk_program_from_stdin("awk -f /dev/stdin"));
        assert!(!awk_program_from_stdin("awk -f transform.awk"));
        assert!(!awk_program_from_stdin("awk '{ print }'"));
    }

    #[test]
    fn control_flow_boundary_includes_short_circuits() {
        // `&&`/`||` conditional assignment is a boundary (reviewer finding #4).
        assert!(control_flow_boundary(r#"[[ -n "$FOO" ]] && arr=()"#));
        assert!(control_flow_boundary("cmd || fallback"));
        // A plain command with a `${...}` expansion is not a boundary.
        assert!(!control_flow_boundary(r#"printf '%s' "${arr[@]}""#));
        assert!(!control_flow_boundary("arr=()"));
    }

    #[test]
    fn control_flow_boundary_includes_scope_exits_and_case_arm_terminators() {
        // A closing brace or paren in command position ends a function body,
        // group, or subshell, so tracked facts must not survive it (#550).
        assert!(control_flow_boundary("}"));
        assert!(control_flow_boundary("  }"));
        assert!(control_flow_boundary(")"));
        assert!(control_flow_boundary("body; }"));
        assert!(control_flow_boundary("} 2>/dev/null"));
        // Case-arm terminators end a branch.
        assert!(control_flow_boundary("printf ok ;;"));
        assert!(control_flow_boundary(": ;&"));
        // The `)` of an initializer, a mid-line command substitution, or a
        // `${...}` expansion is not a scope exit.
        assert!(!control_flow_boundary("arr=( )"));
        assert!(!control_flow_boundary("x=$( cmd )"));
        assert!(!control_flow_boundary("y=$(( 1 + 2 ))"));
        assert!(!control_flow_boundary(r#"printf '%s' "${items[@]}""#));
        // A closer inside a comment or string is inert.
        assert!(!control_flow_boundary("true # }"));
        assert!(!control_flow_boundary("echo '}'"));
    }

    #[test]
    fn set_flag_detection_covers_clusters_and_long_options() {
        assert_eq!(
            set_flags("set -euo pipefail"),
            SetFlags {
                errexit: true,
                nounset: true
            }
        );
        assert_eq!(
            set_flags("set -o errexit"),
            SetFlags {
                errexit: true,
                nounset: false
            }
        );
        assert_eq!(
            set_flags("set -o nounset"),
            SetFlags {
                errexit: false,
                nounset: true
            }
        );
        assert!(set_flags("set -uo pipefail").nounset);
        assert!(!set_flags("echo set -e in a string").errexit);
    }

    #[test]
    fn array_helpers_read_live_code_only() {
        let assignments = array_assignments("items=(); other=(a b)");
        assert_eq!(assignments.len(), 2);
        assert!(assignments[0].empty);
        assert!(!assignments[1].empty);
        let expansions = unguarded_array_expansions(r#"printf '%s' "${items[@]}""#);
        assert_eq!(expansions.len(), 1);
        assert_eq!(expansions[0].0, "items");
        // The `:-`/`+` guarded form is not unguarded.
        assert!(unguarded_array_expansions(r#"printf '%s' "${items[@]:-}""#).is_empty());
        assert!(unguarded_array_expansions(r#"printf '%s' ${items[@]+"${items[@]}"}"#).is_empty());
    }

    #[test]
    fn awk_command_parsing_extracts_separators_and_assignments() {
        let command = parse_awk_command("awk -F ',' -v x=1 'BEGIN{print}'").unwrap();
        assert_eq!(command.field_separator.as_deref(), Some(","));
        assert_eq!(
            command.assignments,
            vec![("x".to_string(), "1".to_string())]
        );
        assert!(parse_awk_command("echo hi").is_none());
    }

    #[test]
    fn awk_analysis_targets_regex_operands_only() {
        // Display-only variable: clean.
        assert!(
            analyze_awk(
                "awk -v label='x' 'BEGIN { print label }'",
                "BEGIN { print label }"
            )
            .option_evidence
            .is_empty()
        );
        // Variable used in a regex with a non-ASCII value: option finding.
        let used = analyze_awk("awk -v re='—' '$0 ~ re'", "$0 ~ re");
        assert_eq!(used.option_evidence, vec!["—".to_string()]);
        // Non-ASCII field separator: option finding; ASCII one: clean.
        assert_eq!(
            analyze_awk("awk -F '—' '{print $1}'", "{print $1}").option_evidence,
            vec!["—".to_string()]
        );
        assert!(
            analyze_awk("awk -F ',' '{print $1}'", "{print $1}")
                .option_evidence
                .is_empty()
        );
        // Non-ASCII regex operand in the program: program finding.
        let program = "BEGIN { if ($0 ~ /ASCII/) print \"テスト\" }";
        assert!(
            analyze_awk("awk 'prog'", program)
                .program_findings
                .is_empty(),
            "ASCII regex with non-ASCII output is clean"
        );
        let match_program = "match($0, \"—\")";
        assert_eq!(
            analyze_awk("awk 'prog'", match_program)
                .program_findings
                .len(),
            1
        );
        let gsub_program = "gsub(\"—\", \"-\", $0)";
        assert_eq!(
            analyze_awk("awk 'prog'", gsub_program)
                .program_findings
                .len(),
            1
        );
        let split_program = "split($0, parts, \"—\")";
        assert_eq!(
            analyze_awk("awk 'prog'", split_program)
                .program_findings
                .len(),
            1
        );
    }

    #[test]
    fn inline_program_extraction_finds_content_and_offset() {
        let (program, offset) = inline_awk_program("awk -v x=1 'BEGIN { print }' file").unwrap();
        assert_eq!(program, "BEGIN { print }");
        assert_eq!(
            &"awk -v x=1 'BEGIN { print }' file"[offset..offset + 5],
            "BEGIN"
        );
        // A `-f` program has no inline operand.
        assert!(inline_awk_program("awk -f prog.awk file").is_none());
        // Multi-line inline: content offset lands on the command line.
        let text = "awk 'BEGIN {\n  print\n}'";
        let (program, offset) = inline_awk_program(text).unwrap();
        assert!(program.starts_with("BEGIN {"));
        assert_eq!(text[..offset].bytes().filter(|b| *b == b'\n').count(), 0);
    }

    #[test]
    fn awk_regex_literal_line_offsets_are_reported() {
        let program = "BEGIN {\n  gsub(\"—\", \"-\", $0)\n}";
        let findings = analyze_awk("awk 'prog'", program).program_findings;
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].0, 1, "regex is on the second program line");
    }

    #[test]
    fn awk_definite_constant_flow_reaches_regex_uses() {
        let findings = |program: &str| analyze_awk("awk 'prog'", program).program_findings;
        // The #550 reproduction: a definite in-program constant reaching a
        // `~` use — the use may sit inside a branch.
        assert_eq!(
            findings(r#"BEGIN { re="—"; if ($0 ~ re) print "match" }"#),
            vec![(0, "—".to_string())]
        );
        // Definite flows into every recognized regex position.
        assert_eq!(
            findings(r#"BEGIN { sep = "—"; FS = sep }"#).len(),
            1,
            "FS flow"
        );
        assert_eq!(
            findings(
                r#"BEGIN { re = "—" }
{ if (match($0, re)) print }"#
            )
            .len(),
            1,
            "match flow"
        );
        assert_eq!(
            findings(r#"BEGIN { re = "—"; gsub(re, "-") }"#).len(),
            1,
            "gsub flow"
        );
        assert_eq!(
            findings(r#"BEGIN { re = "—"; sub(re, "-") }"#).len(),
            1,
            "sub flow"
        );
        assert_eq!(
            findings(r#"BEGIN { re = "—"; n = split($0, parts, re) }"#).len(),
            1,
            "split flow"
        );
        // The finding is anchored to the assignment's line.
        assert_eq!(
            findings("BEGIN {\n  re = \"—\"\n  if ($0 ~ re) print\n}"),
            vec![(1, "—".to_string())]
        );
    }

    #[test]
    fn awk_constant_flow_stays_silent_on_every_ambiguity() {
        let findings = |program: &str| analyze_awk("awk 'prog'", program).program_findings;
        // Display-only value: never used as a regex.
        assert!(findings(r#"BEGIN { msg="—"; print msg }"#).is_empty());
        // Reassignment ambiguity, including compound and increment forms.
        assert!(findings(r#"BEGIN { re="—"; re="x"; if ($0 ~ re) print }"#).is_empty());
        assert!(findings(r#"BEGIN { re="—"; sub(/x/, "y", re); if ($0 ~ re) print }"#).is_empty());
        // Branch-dependent values, brace-less and braced, loops, and ternary.
        assert!(findings(r#"BEGIN { if (c) re="—"; if ($0 ~ re) print }"#).is_empty());
        assert!(findings(r#"BEGIN { if (c) { re="—" } if ($0 ~ re) print }"#).is_empty());
        assert!(findings("BEGIN { if (c)\n    re=\"—\"\n  if ($0 ~ re) print }").is_empty());
        assert!(findings(r#"BEGIN { while (c) re="—"; if ($0 ~ re) print }"#).is_empty());
        assert!(findings(r#"BEGIN { x = c ? (re = "—") : 0; if ($0 ~ re) print }"#).is_empty());
        // A pattern-guarded or bare main-rule action runs per input line, so
        // its assignment is not definite; user function bodies need a caller.
        assert!(
            findings(
                r#"$0 ~ /x/ { re="—" }
$1 ~ re { print }"#
            )
            .is_empty()
        );
        assert!(
            findings(
                r#"{ re="—" }
END { if ($0 ~ re) print }"#
            )
            .is_empty()
        );
        assert!(
            findings(
                r#"function setup() { re="—" }
BEGIN { if ($0 ~ re) print }"#
            )
            .is_empty()
        );
        // Computed values: concatenation and command results.
        assert!(findings(r#"BEGIN { re = "—" tail; if ($0 ~ re) print }"#).is_empty());
        assert!(findings(r#"BEGIN { "cmd" | getline re; if ($0 ~ re) print }"#).is_empty());
        // A use textually before the only assignment never sees the value.
        assert!(
            findings(
                r#"$0 ~ re { n++ }
END { re = "—" }"#
            )
            .is_empty()
        );
        // Comparison operators are not assignments and add no count.
        assert_eq!(
            findings(r#"BEGIN { re="—"; if (re == "—" && $0 ~ re) print }"#).len(),
            1
        );
        // ASCII constants stay clean even when the flow is definite.
        assert!(findings(r#"BEGIN { re="ascii"; if ($0 ~ re) print }"#).is_empty());
    }
}
