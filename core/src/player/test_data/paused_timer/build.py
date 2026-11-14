"""Rebuild the authored timer regression with the repository ASC compiler."""
import argparse
from pathlib import Path
import struct
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("--java", required=True)
parser.add_argument("--asc", required=True, type=Path)
parser.add_argument("--playerglobal", required=True, type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parent
subprocess.run(
    [args.java, "-classpath", str(args.asc), "macromedia.asc.embedding.Main",
     "-AS3", "-import", str(args.playerglobal), str(root / "TimerRoot.as")],
    check=True,
    creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
)


def tag(code, payload=b""):
    return struct.pack("<HI", (code << 6) | 63, len(payload)) + payload


body = b"\x08\x00" + struct.pack("<HH", 60 * 256, 1)
body += tag(69, struct.pack("<I", 8))
body += tag(82, struct.pack("<I", 0) + b"\0" + (root / "TimerRoot.abc").read_bytes())
body += tag(76, struct.pack("<HH", 1, 0) + b"TimerRoot\0")
body += tag(1) + tag(0)
(root / "TimerRoot.swf").write_bytes(b"FWS\x0e" + struct.pack("<I", len(body) + 8) + body)
