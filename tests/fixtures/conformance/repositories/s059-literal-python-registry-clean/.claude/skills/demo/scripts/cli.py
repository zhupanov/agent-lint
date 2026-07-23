import importlib

_REGISTRY = {
    ("session", "setup"): ("handlers.session", "setup"),
}


def dispatch(argv):
    module_name, function_name = _REGISTRY[(argv[0], argv[1])]
    return getattr(importlib.import_module(module_name), function_name)(argv[2:])
