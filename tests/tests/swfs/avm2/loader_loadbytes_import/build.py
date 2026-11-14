"""Build the original minimal fixtures using ASC and a playerglobal import ABC."""

import argparse
from pathlib import Path
import struct
import subprocess
import tempfile


def tag(code, payload):
    return struct.pack("<HI", (code << 6) | 63, len(payload)) + payload


def movie(tags):
    # Zero-sized stage, 24 FPS, one frame, AS3 FileAttributes.
    body = b"\x08\x00" + struct.pack("<HH", 24 * 256, 1)
    body += tag(69, struct.pack("<I", 8))
    body += tags + tag(1, b"") + tag(0, b"")
    return b"FWS\x0a" + struct.pack("<I", 8 + len(body)) + body


def abc_tag(abc):
    return tag(82, struct.pack("<I", 0) + b"\0" + abc)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--asc", required=True, type=Path)
    parser.add_argument("--playerglobal", required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    compiled = {}
    with tempfile.TemporaryDirectory() as directory:
        for name in ("Test", "AfterImport"):
            source = Path(directory) / (name + ".as")
            source.write_bytes((root / source.name).read_bytes())
            subprocess.run(
                [
                    "java", "-classpath", str(args.asc.resolve()),
                    "macromedia.asc.embedding.Main", "-AS3", "-import",
                    str(args.playerglobal.resolve()), str(source),
                ],
                check=True,
            )
            compiled[name] = source.with_suffix(".abc").read_bytes()

    (root / "test.swf").write_bytes(movie(
        abc_tag(compiled["Test"]) + tag(76, struct.pack("<HH", 1, 0) + b"Test\0")
    ))
    (root / "loadable.swf").write_bytes(movie(
        tag(71, b"empty.swf\0\x01\x00\x00\x00") + abc_tag(compiled["AfterImport"])
    ))
    (root / "empty.swf").write_bytes(movie(b""))


if __name__ == "__main__":
    main()
