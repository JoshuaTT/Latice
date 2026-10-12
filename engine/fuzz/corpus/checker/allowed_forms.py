from lattice import EAST
LIMIT = 3
NAMES = ["a", (1, -2)]

def init(me, args):
    pass

def tick(me):
    x = [i * 2 for i in range(3) if i > 0]
    a, b = 1, 2
    y = x[1:2]
    z = x[::-1]
    c = 1 < a <= 2
    n = -a if a else ~b
    while a:
        a -= 1
        if a:
            continue
        elif b:
            break
    me.mem["k"] = {1: [2], "s": None}
    return None

def on_hit(me, attacker):
    pass

def on_hacked(me, attacker):
    pass

def on_radio(me, msg):
    pass

def helper(a: int, b=2):
    return a
