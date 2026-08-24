from pathlib import Path

Path("PROJECT_RUNTIME_EXECUTED").write_text("poison", encoding="utf-8")
raise RuntimeError("Codeclew must never execute or import fixture code")
