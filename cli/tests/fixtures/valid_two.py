def init(me, args):
    me.mem["n"] = 0


def tick(me):
    me.mem["n"] = me.mem["n"] + 1


def on_hit(me, attacker):
    pass
