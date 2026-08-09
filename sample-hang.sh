#!/bin/sh
# Grab a stack from the Harmonigraph plug-in host while it is hung.
#
# Bitwig logs "Plug-in host is not responding" and then SIGKILLs the process
# about thirty seconds later, so the stack is gone by the time anyone looks:
# a SIGKILL leaves no crash report, and a hang leaves no panic in engine.log
# either. This watches the log and samples the process inside that window.
#
# Two samples, because the question is usually "is it stuck or is it spinning"
# and one sample cannot tell them apart: a thread parked in the same lock in
# both is stuck, a thread that has moved is working.
#
#   sh sample-hang.sh &        # start, from the repo root
#   pkill -f sample-hang.sh    # stop
#
# Writes to ~/harmonigraph-hangs/.

out="$HOME/harmonigraph-hangs"
log="$HOME/Library/Logs/Bitwig/BitwigStudio.log"
mkdir -p "$out"

echo "watching $log -> $out"

# -F rather than -f: Bitwig rotates this file on restart, and a watcher holding
# the old inode would sit quiet through every hang after the first.
tail -n 0 -F "$log" | while IFS= read -r line; do
	case "$line" in
	*"Plug-in host is not responding"*) ;;
	*) continue ;;
	esac
	# The host processes are one per vendor and the notice does not name one,
	# so the vendor string is what picks Harmonigraph's out of the four.
	pid=$(pgrep -f "BitwigPluginHost.*host Yan-Han" | head -1)
	if [ -z "$pid" ]; then
		echo "no Yan-Han plug-in host running"
		continue
	fi
	stamp=$(date +%Y%m%d-%H%M%S)
	echo "hang: sampling pid $pid -> $out/hang-$stamp-{a,b}.txt"
	sample "$pid" 3 -f "$out/hang-$stamp-a.txt" >/dev/null 2>&1
	sample "$pid" 3 -f "$out/hang-$stamp-b.txt" >/dev/null 2>&1
	echo "hang: sampled $stamp"
done
