"""Bounded stdio-to-loopback transport for the isolated native generic adapter."""
import argparse, json, signal, sys, time, urllib.request
from pathlib import Path
import sys
sys.path.insert(0,str(Path(__file__).resolve().parent))
from finite_cli import _json_loads, _canonical_json_bytes, JobError
MAX_REQUEST=64*1024*1024

def main():
    p=argparse.ArgumentParser(); p.add_argument('--token-file',type=Path,required=True); a=p.parse_args()
    started=time.monotonic(); raw=sys.stdin.buffer.read(MAX_REQUEST+1)
    if not 0<len(raw)<=MAX_REQUEST: raise JobError('bounded native input required')
    job=_json_loads(raw); cap=job['cap']; timeout=cap['timeoutMs']/1000; deadline=started+timeout
    signal.signal(signal.SIGTERM,lambda *_:sys.exit(143)); signal.signal(signal.SIGINT,lambda *_:sys.exit(130))
    token=a.token_file.read_text().strip()
    req=urllib.request.Request('http://127.0.0.1:55239/invoke',raw,method='POST',headers={'Content-Type':'application/json',
        'X-Private-Bridge-Token':token,'X-Codeclew-Deadline-Monotonic':str(deadline)})
    with urllib.request.urlopen(req,timeout=max(.001,deadline-time.monotonic())) as response:
        body=response.read(cap['outputBytes']+1)
    if time.monotonic()>=deadline or len(body)>cap['outputBytes']: raise JobError('bounded response deadline/bytes exceeded')
    value=_json_loads(body)
    if set(value)!={'schema','invocation','role','model','usage','result'} or value['schema']!='codeclew-documentation-agent-result/1.0' or any(value[k]!=job[k] for k in ['invocation','role','model']):
        raise JobError('native response correlation mismatch')
    sys.stdout.buffer.write(body)

if __name__=='__main__': main()
