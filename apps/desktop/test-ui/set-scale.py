"""Sets the scale of headless mutter's virtual monitor via org.gnome.Mutter.DisplayConfig.

    set-scale.py          # print the monitor and its supported scales
    set-scale.py 1.25     # apply the closest supported scale (temporary config)

Used by run.sh when SPECTO_SCALE is set.
"""
import sys
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
def call(method, args, rtype):
    return bus.call_sync("org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig",
        "org.gnome.Mutter.DisplayConfig", method, args, GLib.VariantType(rtype) if rtype else None,
        Gio.DBusCallFlags.NONE, -1, None)
state = call("GetCurrentState", None, "(ua((ssss)a(siiddada{sv})a{sv})a(iiduba(ssss)a{sv})a{sv})")
serial, monitors, logical, props = state.unpack()
spec, modes, mprops = monitors[0]
cur = [m for m in modes if m[6].get("is-current")][0]
if len(sys.argv) == 1:
    print("monitor", spec, "mode", cur[0], cur[1], "x", cur[2], "supported scales", [round(s, 4) for s in cur[5]])
    print("logical", logical, "props", props)
    sys.exit()
want = float(sys.argv[1])
scale = min(cur[5], key=lambda s: abs(s - want))
cfg = GLib.Variant("(uua(iiduba(ssa{sv}))a{sv})", (serial, 1, [(0, 0, scale, 0, True, [(spec[0], cur[0], {})])], {}))
call("ApplyMonitorsConfig", cfg, None)
print(f"applied scale {scale}")
