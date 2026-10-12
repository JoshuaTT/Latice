from lattice import Pos, guard

def tick(me):
    threat = team.board.get("threat")
    if threat and me.now - threat["t"] < 50:
        target = Pos(threat["x"], threat["y"])
        foe = me.nearest_enemy(within=2)
        if foe and foe.firewall <= 2:
            me.hack(foe, payload="warden")
        else:
            me.step_toward(target)
    else:
        guard(me, team.board.get("home"))
