#!/usr/bin/env python3
"""Generate a bounded 40-service corpus and execute explicit runtime qualification."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True)+"\n")


def generate(spec, output):
    required={"schema","services","largeServiceEndpoints","otherEndpointCounts","scopePaddingBytes","maximumServiceBytes","maximumCorpusBytes","maximumChangedRepositories"}
    if set(spec)!=required or spec["schema"]!="codeclew-documentation-qualification-corpus/1.0":
        raise ValueError("unsupported corpus specification")
    if spec["services"]!=40 or spec["largeServiceEndpoints"]!=40 or spec["maximumChangedRepositories"]!=4:
        raise ValueError("qualification requires the declared 40-service/40-endpoint/four-repository boundary")
    if any(type(n)is not int or not 1<=n<=40 for n in spec["otherEndpointCounts"]) or not spec["otherEndpointCounts"]:
        raise ValueError("invalid endpoint counts")
    if any(type(n)is not int or not 0<=n<=65536 for n in spec["scopePaddingBytes"]) or not spec["scopePaddingBytes"]:
        raise ValueError("invalid scope sizes")
    if not 1<=spec["maximumServiceBytes"]<=1048576 or not 1<=spec["maximumCorpusBytes"]<=16*1048576:
        raise ValueError("invalid corpus byte bounds")
    output=Path(output)
    if output.exists() and any(output.iterdir()):
        raise ValueError("corpus output must be empty")
    output.mkdir(parents=True,exist_ok=True)
    rows=[]
    total=0
    for index in range(40):
        name=["orders","other"][index] if index<2 else f"service{index:02}"
        endpoints=40 if index==0 else spec["otherEndpointCounts"][index%len(spec["otherEndpointCounts"]) ]
        methods=['@GetMapping("/reserve") public int reserve(int quantity) { return normalize(quantity); }']
        methods += [f'@GetMapping("/route-{n}") public int endpoint{n}(int quantity) {{ return normalize(quantity); }}' for n in range(1,endpoints)]
        source='import org.springframework.web.bind.annotation.GetMapping;\npublic class Orders {\n'+'\n'.join(methods)+'\nprivate int normalize(int quantity) { return quantity; }\n}\n'
        padding=spec["scopePaddingBytes"][index%len(spec["scopePaddingBytes"]) ]
        files={"Orders.java":source,"application.properties":f"fixture.service={name}\nfixture.limit=10\n"+"#"+"p"*padding+"\n",
               "openapi.json":json.dumps({"openapi":"3.0.3","info":{"title":name,"version":"1"},"paths":{"/reserve":{"get":{"operationId":"reserve","responses":{"200":{"description":"Quantity"}}}}}},sort_keys=True)+"\n"}
        size=sum(len(content.encode()) for content in files.values())
        total+=size
        if size>spec["maximumServiceBytes"] or total>spec["maximumCorpusBytes"]:
            raise ValueError("generated scope exceeds explicit byte bound")
        for filename,content in files.items():
            path=output/name/filename;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(content)
        rows.append({"id":name,"endpoints":endpoints,"files":len(files),"bytes":size,
                     "contentDigest":"sha256:"+hashlib.sha256(json.dumps(files,sort_keys=True).encode()).hexdigest()})
    manifest={"schema":"codeclew-documentation-generated-corpus/1.0","services":rows,"files":sum(r["files"] for r in rows),"bytes":total,"bounds":spec}
    save(output/"corpus.json",manifest)
    return manifest


def run(fixture_root, output):
    output=Path(output).resolve()
    if output.exists() and any(output.iterdir()):
        raise ValueError("qualification output must be new or empty")
    output.mkdir(parents=True,exist_ok=True)
    spec=json.loads((Path(fixture_root)/"corpus.json").read_text())
    corpus=generate(spec,output/"corpus")
    environment=os.environ.copy()
    environment["CODECLEW_DOCSYS_QUALIFICATION"]=str(output/"runtime.json")
    environment["CODECLEW_DOCSYS_CORPUS"]=str(output/"corpus")
    revision=subprocess.check_output(["git","rev-parse","HEAD"],cwd=ROOT,text=True).strip()
    dirty=subprocess.check_output(["git","diff","--no-ext-diff","--binary"],cwd=ROOT)
    untracked=subprocess.check_output(["git","ls-files","--others","--exclude-standard","-z"],cwd=ROOT).decode().split("\0")
    untracked_inputs={p:"sha256:"+hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in untracked if p and (ROOT/p).is_file() and not (ROOT/p).is_symlink()}
    started=time.monotonic()
    with (output/"execution.log").open("wb") as stream:
        process=subprocess.run(["cargo","test","--locked","-p","clew","--test","documentation_system",
                                "docsys_t15_forty_service_qualification","--","--exact","--ignored","--test-threads=1"],
                               cwd=ROOT,env=environment,stdout=stream,stderr=subprocess.STDOUT)
    runtime=json.loads((output/"runtime.json").read_text()) if (output/"runtime.json").exists() else None
    memory=resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    report={"schema":"codeclew-documentation-runtime-qualification/1.0","status":"PASSED" if process.returncode==0 and runtime and runtime.get("status")=="PASSED" else "FAILED",
            "sourceRevision":revision,"workingDiffDigest":"sha256:"+hashlib.sha256(dirty).hexdigest(),"untrackedInputDigests":untracked_inputs,
            "corpus":{"services":len(corpus["services"]),"files":corpus["files"],"bytes":corpus["bytes"],"bounds":spec},
            "execution":{"exitCode":process.returncode,"wallSeconds":time.monotonic()-started,
                         "peakChildResidentBytes":memory if sys.platform=="darwin" else memory*1024,
                         "memoryBoundary":"Maximum child RSS across build/test processes; not simultaneous total memory."},
            "runtime":runtime,
            "limitations":["Deterministic source-syntax workload; not 40 concurrent compiler builds.",
                            "No model quality or actual GitLab qualification is inferred.",
                            "Capture cold/warm refers to documentation evidence, not an OS cache flush.",
                            "Freshness qualification is limited to explicitly mutated source and dependency boundaries."]}
    save(output/"results.json",report)
    return report


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture-root",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--generate-only",action="store_true")
    args=parser.parse_args()
    try:
        result=generate(json.loads((args.fixture_root/"corpus.json").read_text()),args.output) if args.generate_only else run(args.fixture_root,args.output)
        print(json.dumps(result))
        return 0 if args.generate_only or result["status"]=="PASSED" else 1
    except (ValueError,OSError,subprocess.SubprocessError) as error:
        print(json.dumps({"status":"FAILED","reason":str(error)}))
        return 1


if __name__=="__main__":
    raise SystemExit(main())
