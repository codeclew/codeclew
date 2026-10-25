#!/usr/bin/env python3
"""Record only initial/default Work pages using {} then cursor-only selections."""
from pathlib import Path
import runpy

if __name__ == "__main__":
    runpy.run_path(str(Path(__file__).with_name("collect_proposal.py")),
                  run_name="__main__", init_globals={"DEFAULT_COLLECTION_KIND": "work-initial"})
