def init(me, args):
    pass


def tick(me):
    open("secrets.txt")  # noqa: SIM115 - fixture: intentionally banned code
