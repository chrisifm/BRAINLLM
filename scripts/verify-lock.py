#!/usr/bin/env python3
"""Acquire one POSIX Cargo-target lease, then replace this process with the verification command."""
import argparse
import errno
import json
import os
from pathlib import Path
import stat
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    targets = parser.add_mutually_exclusive_group(required=True)
    targets.add_argument("--target")
    targets.add_argument("--cargo-target", action="store_true")
    parser.add_argument("--check-held", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not args.check_held and not command or args.check_held and command:
        parser.error("provide a command, or --check-held without a command")
    try:
        import fcntl
    except ImportError:
        print("Full verification needs POSIX file locking; use WSL or the native Windows diagnostic workflow", file=sys.stderr)
        return 2
    if args.cargo_target:
        metadata = subprocess.run(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=120, check=True)
        if len(metadata.stdout) > 4 * 1024 * 1024:
            raise ValueError("oversized Cargo metadata")
        directory = json.loads(metadata.stdout).get("target_directory")
        if not isinstance(directory, str) or not directory:
            raise ValueError("missing Cargo target")
        target = Path(directory).resolve()
    else:
        target = Path(args.target).resolve()
    if args.check_held:
        try:
            expected = os.stat(target / ".brain-verify.lock", follow_symlinks=False)
            actual = os.fstat(0)
            if not stat.S_ISREG(expected.st_mode) or (actual.st_dev, actual.st_ino) != (expected.st_dev, expected.st_ino):
                return 1
            fcntl.flock(0, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return 0
        except OSError:
            return 1
    target.mkdir(parents=True, exist_ok=True)
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
    with os.fdopen(os.open(target / ".brain-verify.lock", flags, 0o600), "r+b") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            if error.errno not in (errno.EACCES, errno.EAGAIN):
                raise
            print("verify: another verification owns this Cargo target; wait for it to finish or cancel that owned run before retrying", file=sys.stderr)
            return 75
        if not stat.S_ISREG(os.fstat(lock.fileno()).st_mode) or os.fstat(lock.fileno()).st_size:
            print("verify: expected an empty regular target lock file; inspect existing target metadata", file=sys.stderr)
            return 2
        # Verification is noninteractive. The empty stdin descriptor carries the kernel lock through exec and
        # ordinary children; no supervisor, PID-based cleanup, stale-lock deletion or extra inherited fd is needed.
        os.dup2(lock.fileno(), 0, inheritable=True)
        env = {**os.environ, "CARGO_TARGET_DIR": str(target), "BRAIN_VERIFY_LOCK_HELD": "1"}
        os.execvpe(command[0], command, env)
    return 2


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError):
        print("verify: cannot acquire or execute the verification; check Cargo metadata, target permissions and command availability", file=sys.stderr)
        sys.exit(2)
