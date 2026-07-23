import importlib

_REGISTRY = {
    ("alpha", "run"): ("larch.commands.alpha", "run"),
    ("beta", "run"): ("larch.commands.beta", "run"),
}


def main(argv):
    module_name, function_name = _REGISTRY[(argv[0], argv[1])]
    return getattr(importlib.import_module(module_name), function_name)(argv[2:])
