def setup(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("--prefix")
    parser.add_argument("--skip-preflight", action="store_true")
