"""Machine-wide coordination with the concurrent Otonal audit; never kills owners."""
import fcntl, subprocess, sys, datetime
with open('/private/tmp/yan-deep-audits-expensive.lock', 'a') as lock:
    print('Waiting for shared audit lock', flush=True)
    fcntl.flock(lock, fcntl.LOCK_EX)
    print('Acquired shared audit lock', datetime.datetime.now(datetime.timezone.utc).isoformat(), flush=True)
    raise SystemExit(subprocess.call(sys.argv[1:]))
