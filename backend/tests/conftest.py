import os
import sys
import tempfile
from pathlib import Path

# Isolated data dir per test session, set before the app modules are imported.
os.environ["TP_DATA_DIR"] = tempfile.mkdtemp(prefix="tp-test-")
os.environ["TP_SNAPSHOT_INTERVAL"] = "0"
os.environ.pop("GEMINI_API_KEY", None)
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
