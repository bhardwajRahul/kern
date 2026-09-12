#!/bin/sh
# Give a container exactly one hardware device - and nothing else.
#
# kern binds only the device you name into the box; every other host device
# stays absent (deny-by-default). Handy on edge boards: a sensor on i2c, a
# serial MCU, or a SPI peripheral - exposed to one workload, kept away from the
# rest of the system. Device access is node-granular (a whole /dev node), which
# is a real kernel boundary; see SECURITY.md for the GPIO-line caveat.
set -eu
kern="${KERN:-kern}"
# WHICH BINARY IS THIS. Printed to stderr on every run, because `${KERN:-kern}` silently
# resolves to whatever `kern` is on PATH: a validation that forgets to set KERN measures the
# INSTALLED release while believing it measured the build under test, and reports green for
# code that never ran. A wrong binary has to be visible in the output, not inferred from it.
printf '# using %s (%s)\n' "$(command -v "$kern" || echo "$kern")" "$("$kern" --version 2>&1 | head -1)" >&2


# Pick a device that actually exists on this host, and the matching profile field.
#
# GLOBBED, NOT A FIXED LIST, and a Raspberry Pi 5 is why. The list here named `/dev/spidev0.0`, and
# the Pi 5 puts its SPI on the RP1 as `/dev/spidev10.0`: MEASURED on one, this example printed "No
# i2c/serial/spi device on this host to demo with" on the board that is the best hardware for the
# demo it is refusing to run. i2c also needs enabling in raspi-config, so a stock Pi has no
# `/dev/i2c-1` either. A glob asks the host what it HAS instead of asserting what it should be called.
dev=""; field=""
for cand in /dev/i2c-* /dev/spidev* /dev/ttyUSB* /dev/ttyACM*; do
  [ -e "$cand" ] || continue
  case "$cand" in
    /dev/i2c-*)   field="i2c" ;;
    /dev/tty*)    field="uart" ;;
    /dev/spidev*) field="spi" ;;
  esac
  dev="$cand"; break
done

# AND THE FALLBACK EVERY LINUX BOARD HAS: a LED in sysfs. It demonstrates the same rule through the
# other kind of grant - a sysfs directory rather than a `/dev` node - so the example runs on a host
# with no bus wired up at all, which is most laptops. `leds` takes the NAME, never a path.
led=""
if [ -z "$dev" ] && [ -d /sys/class/leds ]; then
  led="$(ls -1 /sys/class/leds 2>/dev/null | head -1)"
fi

if [ -z "$dev" ] && [ -z "$led" ]; then
  echo "No i2c/serial/spi device and no sysfs LED on this host to demo with."
  echo "On a Pi/Jetson/Arduino you'd expose e.g. an i2c sensor:"
  echo "    [[vgpio]]"
  echo "    name = \"sensor\""
  echo "    i2c  = [\"/dev/i2c-1\"]"
  echo "  then:  kern box app --image alpine vgpio:sensor -- ./read-sensor"
  exit 0
fi

cfg="$(mktemp -d)/kern.toml"
if [ -n "$dev" ]; then
  grant="$field = [\"$dev\"]"; shown="$dev"
else
  grant="leds = [\"$led\"]"; shown="the LED $led (sysfs)"
fi
cat > "$cfg" <<EOF
[[vgpio]]
name = "sensor"
# Required: the host resource this profile slices.
backend = "host"
$grant
EOF

echo "==> exposing only $shown into the box (profile vgpio:sensor):"
echo
"$kern" box hw --image alpine --config "$cfg" vgpio:sensor -- sh -c '
  echo "  device nodes in the box:";
  ls -1 /dev | grep -E "^(i2c-|tty(USB|ACM|S)|spidev|gpiochip)" | sed "s/^/    /" || true
  echo "  LEDs in the box:";
  leds="$(ls -1 /sys/class/leds 2>/dev/null)";
  if [ -n "$leds" ]; then echo "$leds" | sed "s/^/    /"; else echo "    none"; fi
  echo;
  echo "  host disks in the box?";
  if ls /dev/nvme* /dev/sd* /dev/mmcblk* 2>/dev/null | grep -q .; then
    echo "    LEAK: host disks visible"; else echo "    none - host storage is not exposed"; fi
'

echo
echo "The box saw one device and nothing else. Everything is discarded on exit."
rm -rf "$(dirname "$cfg")"
