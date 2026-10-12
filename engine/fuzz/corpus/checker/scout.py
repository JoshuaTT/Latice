from lattice import EAST, turn_right

def init(me, args):
    me.mem["dir"] = args.get("dir", EAST)

def tick(me):
    for e in me.sense(radius=5).enemies():
        me.mark(e.pos, "danger", ttl=150)
        team.board.put("threat", {"x": e.pos.x, "y": e.pos.y, "t": me.now})
    if me.blocked(me.mem["dir"]):
        me.mem["dir"] = turn_right(me.mem["dir"])
    me.move(me.mem["dir"])
