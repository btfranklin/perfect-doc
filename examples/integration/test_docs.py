"""Call the installed Perfect Doc command from pytest."""

import os
import subprocess
from pathlib import Path


def test_documentation_structure() -> None:
    example = Path(__file__).resolve().parent
    command = os.environ.get("PERFECT_DOC", "perfect-doc")
    subprocess.run([command, "check", "docs"], cwd=example, check=True)
