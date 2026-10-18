# Runs remote_cases.py once per item, each in its own process (as a single request would),
# eight at a time, and reports every item's outcome in order. One SSH session, one engine
# check and one lock cover the whole batch.
import concurrent.futures, json, re, subprocess, sys

SCRIPT = json.loads(sys.argv[1])
JOBS = json.loads(sys.argv[2])

def failure(stderr, code):
    lines = stderr.rstrip().splitlines()
    for index in range(len(lines) - 1, -1, -1):
        if re.match(r'[A-Za-z_][\w.]*: ', lines[index]):
            return '\n'.join(lines[index:])
    return stderr.strip()[-2000:] or 'exit status %d' % code

def run(job):
    try:
        done = subprocess.run([sys.executable, '-c', SCRIPT, job], capture_output=True, text=True)
    except Exception as error:
        return {'error': str(error)}
    if done.returncode == 0:
        try:
            return {'ok': json.loads(done.stdout)}
        except ValueError:
            return {'error': 'invalid server response: ' + done.stdout.strip()[-500:]}
    return {'error': failure(done.stderr, done.returncode)}

with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    results = list(pool.map(run, JOBS))
print(json.dumps({'results': results}))
