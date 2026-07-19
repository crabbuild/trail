#!/usr/bin/env python3
"""Clone a directory tree with clonefile(2), never a byte-copy fallback."""

from __future__ import annotations

import ctypes
import errno
import hashlib
import json
import os
import stat
import sys
from pathlib import Path
from typing import Any


CLONE_NOFOLLOW = 0x0001
LIBC = ctypes.CDLL(None, use_errno=True)
CLONEFILE = LIBC.clonefile
CLONEFILE.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_int]
CLONEFILE.restype = ctypes.c_int


class CloneTree:
    def __init__(self, source: Path, destination: Path, manifest: Path) -> None:
        self.source = source
        self.destination = destination
        self.manifest = manifest
        self.primaries: dict[tuple[int, int], tuple[Path, os.stat_result]] = {}
        self.directory_metadata: list[tuple[Path, Path, os.stat_result]] = []
        self.entries: list[dict[str, Any]] = []
        self.clones: list[dict[str, Any]] = []
        self.counters = {
            "directories": 0,
            "symlinks": 0,
            "regular_paths": 0,
            "unique_regular_files": 0,
            "clonefile_calls_attempted": 0,
            "clonefile_calls_succeeded": 0,
            "hardlinks_created": 0,
            "special_entries_rejected": 0,
            "byte_copy_calls": 0,
        }
        self.value: dict[str, Any] = {
            "schema_version": 1,
            "status": "RUNNING",
            "clone_api": "clonefile(2)",
            "byte_copy_fallback": False,
            "source": str(source),
            "destination": str(destination),
            "source_device": os.lstat(source).st_dev,
            "destination_device": os.lstat(destination).st_dev,
            "counters": self.counters,
            "entries": self.entries,
            "clonefile_calls": self.clones,
        }

    @staticmethod
    def fingerprint(metadata: os.stat_result) -> tuple[int, ...]:
        return (
            metadata.st_dev,
            metadata.st_ino,
            metadata.st_mode,
            metadata.st_nlink,
            metadata.st_size,
            metadata.st_mtime_ns,
            metadata.st_ctime_ns,
        )

    def assert_unchanged(self, path: Path, before: os.stat_result) -> None:
        after = os.lstat(path)
        if self.fingerprint(after) != self.fingerprint(before):
            raise RuntimeError(f"source entry raced during clone: {path}")

    def clone_regular(self, source: Path, destination: Path, relative: str,
                      metadata: os.stat_result) -> None:
        self.counters["regular_paths"] += 1
        key = (metadata.st_dev, metadata.st_ino)
        primary = self.primaries.get(key)
        if primary is not None:
            primary_path, primary_metadata = primary
            if self.fingerprint(primary_metadata) != self.fingerprint(metadata):
                raise RuntimeError(f"hardlink metadata changed during clone: {source}")
            os.link(primary_path, destination, follow_symlinks=False)
            self.counters["hardlinks_created"] += 1
            self.assert_unchanged(source, metadata)
            if os.lstat(primary_path).st_ino != os.lstat(destination).st_ino:
                raise RuntimeError(f"destination hardlink identity mismatch: {relative}")
            self.entries.append({"path": relative, "type": "regular", "action": "hardlink",
                                 "primary": str(primary_path.relative_to(self.destination))})
            return

        self.counters["unique_regular_files"] += 1
        self.counters["clonefile_calls_attempted"] += 1
        ctypes.set_errno(0)
        result = CLONEFILE(os.fsencode(source), os.fsencode(destination), CLONE_NOFOLLOW)
        call: dict[str, Any] = {
            "path": relative,
            "source_device": metadata.st_dev,
            "size": metadata.st_size,
            "success": result == 0,
        }
        if result != 0:
            error_number = ctypes.get_errno() or errno.EIO
            call.update(errno=error_number, errno_name=errno.errorcode.get(error_number, "UNKNOWN"))
            self.clones.append(call)
            raise OSError(error_number, "clonefile(2) failed", str(source), str(destination))
        self.counters["clonefile_calls_succeeded"] += 1
        destination_metadata = os.lstat(destination)
        call["destination_device"] = destination_metadata.st_dev
        call["destination_size"] = destination_metadata.st_size
        self.clones.append(call)
        self.assert_unchanged(source, metadata)
        if not stat.S_ISREG(destination_metadata.st_mode):
            raise RuntimeError(f"clonefile destination is not regular: {relative}")
        if destination_metadata.st_dev != metadata.st_dev or destination_metadata.st_size != metadata.st_size:
            raise RuntimeError(f"clonefile destination device/size mismatch: {relative}")
        os.chmod(destination, stat.S_IMODE(metadata.st_mode))
        os.utime(destination, ns=(metadata.st_atime_ns, metadata.st_mtime_ns), follow_symlinks=False)
        self.primaries[key] = (destination, metadata)
        self.entries.append({"path": relative, "type": "regular", "action": "clonefile"})

    def visit(self, source_directory: Path, destination_directory: Path) -> None:
        directory_before = os.lstat(source_directory)
        with os.scandir(source_directory) as stream:
            entries = sorted(stream, key=lambda entry: os.fsencode(entry.name))
        for entry in entries:
            source = Path(entry.path)
            relative = source.relative_to(self.source).as_posix()
            destination = self.destination / relative
            metadata = os.lstat(source)
            if os.path.lexists(destination):
                raise FileExistsError(errno.EEXIST, "destination entry already exists", str(destination))
            if stat.S_ISDIR(metadata.st_mode) and not stat.S_ISLNK(metadata.st_mode):
                os.mkdir(destination, 0o700)
                self.counters["directories"] += 1
                self.entries.append({"path": relative, "type": "directory", "action": "mkdir"})
                self.visit(source, destination)
                self.directory_metadata.append((source, destination, metadata))
            elif stat.S_ISREG(metadata.st_mode):
                self.clone_regular(source, destination, relative, metadata)
            elif stat.S_ISLNK(metadata.st_mode):
                target = os.readlink(source)
                os.symlink(target, destination)
                os.utime(destination, ns=(metadata.st_atime_ns, metadata.st_mtime_ns),
                         follow_symlinks=False)
                self.assert_unchanged(source, metadata)
                self.counters["symlinks"] += 1
                self.entries.append({"path": relative, "type": "symlink", "action": "symlink",
                                     "target_sha256": hashlib.sha256(os.fsencode(target)).hexdigest()})
            else:
                self.counters["special_entries_rejected"] += 1
                raise RuntimeError(f"unsupported special source entry: {relative}")
        self.assert_unchanged(source_directory, directory_before)

    def restore_directories(self) -> None:
        root_metadata = os.lstat(self.source)
        for source, destination, metadata in reversed(self.directory_metadata):
            self.assert_unchanged(source, metadata)
            os.chmod(destination, stat.S_IMODE(metadata.st_mode))
            os.utime(destination, ns=(metadata.st_atime_ns, metadata.st_mtime_ns),
                     follow_symlinks=False)
        os.chmod(self.destination, stat.S_IMODE(root_metadata.st_mode))
        os.utime(self.destination, ns=(root_metadata.st_atime_ns, root_metadata.st_mtime_ns),
                 follow_symlinks=False)
        self.assert_unchanged(self.source, root_metadata)

    def run(self) -> None:
        self.visit(self.source, self.destination)
        self.restore_directories()
        source_rows = inventory(self.source)
        destination_rows = inventory(self.destination)
        source_digest = digest_json(source_rows)
        destination_digest = digest_json(destination_rows)
        self.value.update(
            status="PASS",
            source_tree_sha256=source_digest,
            destination_tree_sha256=destination_digest,
            source_inventory_sha256=source_digest,
            destination_inventory_sha256=destination_digest,
        )
        if source_rows != destination_rows:
            raise RuntimeError("source and cloned destination inventories differ")
        if self.counters["clonefile_calls_attempted"] != self.counters["clonefile_calls_succeeded"]:
            raise RuntimeError("not every clonefile call succeeded")
        if self.counters["regular_paths"] != (
            self.counters["clonefile_calls_succeeded"] + self.counters["hardlinks_created"]
        ):
            raise RuntimeError("regular-file accounting is incomplete")

    def fail(self, error: BaseException) -> None:
        self.value["status"] = "FAIL"
        failure: dict[str, Any] = {"type": type(error).__name__, "message": str(error)}
        if isinstance(error, OSError) and error.errno is not None:
            failure.update(errno=error.errno, errno_name=errno.errorcode.get(error.errno, "UNKNOWN"))
        self.value["failure"] = failure

    def write_manifest(self) -> None:
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        if hasattr(os, "O_NOFOLLOW"):
            flags |= os.O_NOFOLLOW
        descriptor = os.open(self.manifest, flags, 0o600)
        try:
            payload = (json.dumps(self.value, sort_keys=True, separators=(",", ":")) + "\n").encode()
            view = memoryview(payload)
            while view:
                written = os.write(descriptor, view)
                if written <= 0:
                    raise OSError(errno.EIO, "short manifest write")
                view = view[written:]
            os.fsync(descriptor)
        finally:
            os.close(descriptor)


def digest_file(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb", buffering=0) as stream:
        while chunk := stream.read(1024 * 1024):
            value.update(chunk)
    return value.hexdigest()


def inventory(root: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    inode_paths: dict[tuple[int, int], list[str]] = {}

    def visit(directory: Path) -> None:
        with os.scandir(directory) as stream:
            entries = sorted(stream, key=lambda entry: os.fsencode(entry.name))
        for entry in entries:
            path = Path(entry.path)
            relative = path.relative_to(root).as_posix()
            metadata = os.lstat(path)
            row: dict[str, Any] = {
                "path": relative,
                "mode": stat.S_IMODE(metadata.st_mode),
                "mtime_ns": metadata.st_mtime_ns,
            }
            if stat.S_ISDIR(metadata.st_mode) and not stat.S_ISLNK(metadata.st_mode):
                row.update(type="directory", size=None, digest=None)
                rows.append(row)
                visit(path)
                continue
            if stat.S_ISREG(metadata.st_mode):
                row.update(type="regular", size=metadata.st_size, digest=digest_file(path))
                inode_paths.setdefault((metadata.st_dev, metadata.st_ino), []).append(relative)
            elif stat.S_ISLNK(metadata.st_mode):
                target = os.readlink(path)
                row.update(type="symlink", size=len(os.fsencode(target)),
                           digest=hashlib.sha256(os.fsencode(target)).hexdigest())
            else:
                raise RuntimeError(f"unsupported special entry during inventory: {relative}")
            rows.append(row)

    visit(root)
    groups = {path: paths[0] for paths in inode_paths.values() if len(paths) > 1 for path in paths}
    for row in rows:
        if row["path"] in groups:
            row["hardlink_group"] = groups[row["path"]]
    return rows


def digest_json(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def canonical_existing_directory(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute() or os.path.normpath(value) != value or os.path.realpath(value) != value:
        raise ValueError(f"directory must be absolute and canonical: {value}")
    metadata = os.lstat(path)
    if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
        raise ValueError(f"not a real directory: {value}")
    return path


def validate(source_value: str, destination_value: str, manifest_value: str) -> tuple[Path, Path, Path]:
    source = canonical_existing_directory(source_value)
    destination = canonical_existing_directory(destination_value)
    manifest = Path(manifest_value)
    if not manifest.is_absolute() or os.path.normpath(manifest_value) != manifest_value:
        raise ValueError("manifest path must be absolute and normalized")
    if os.path.lexists(manifest):
        raise FileExistsError(errno.EEXIST, "manifest already exists", str(manifest))
    canonical_existing_directory(str(manifest.parent))
    with os.scandir(destination) as stream:
        if next(stream, None) is not None:
            raise ValueError("destination must be empty")
    if source == destination or source in destination.parents or destination in source.parents:
        raise ValueError("source and destination must not overlap")
    if os.lstat(source).st_dev != os.lstat(destination).st_dev:
        raise OSError(errno.EXDEV, "source and destination devices differ")
    return source, destination, manifest


def main() -> int:
    if len(sys.argv) != 4:
        print(f"usage: {sys.argv[0]} SOURCE DESTINATION MANIFEST", file=sys.stderr)
        return 64
    try:
        source, destination, manifest = validate(*sys.argv[1:])
    except BaseException as error:
        print(f"apfs-clone-tree: {error}", file=sys.stderr)
        return 64
    operation = CloneTree(source, destination, manifest)
    try:
        operation.run()
    except BaseException as error:
        operation.fail(error)
        operation.write_manifest()
        print(f"apfs-clone-tree: {error}", file=sys.stderr)
        return 74
    operation.write_manifest()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
