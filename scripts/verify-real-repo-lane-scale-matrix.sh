#!/usr/bin/env bash
# Run the 64/128 real-repository scale matrix only in independent APFS copies.
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)
APFS_CLONE_TREE_HELPER=$SCRIPT_DIR/apfs-clone-tree.py

TRAIL_BIN=${TRAIL_BIN:-}
TRAIL_SCALE_REPO=${TRAIL_SCALE_REPO:-}
TRAIL_SCALE_MATRIX_OUTPUT=${TRAIL_SCALE_MATRIX_OUTPUT:-}
TRAIL_SCALE_INNER_HARNESS=${TRAIL_SCALE_INNER_HARNESS:-$SCRIPT_DIR/verify-real-repo-lane-scale.sh}
TRAIL_SCALE_MATRIX_RUN_ID=${TRAIL_SCALE_MATRIX_RUN_ID:-}
TRAIL_SCALE_MATRIX_PUBLISH=${TRAIL_SCALE_MATRIX_PUBLISH:-0}
TRAIL_SCALE_FILES_PER_LANE=${TRAIL_SCALE_FILES_PER_LANE:-50}
TRAIL_SCALE_FAULT_PHASE=${TRAIL_SCALE_FAULT_PHASE:-all}
TRAIL_SCALE_LATENCY_CEILING_SECONDS=${TRAIL_SCALE_LATENCY_CEILING_SECONDS:-120}
active_inner_pid=
active_inner_run_dir=
active_inner_pgid_file=

die() {
  echo "verify-real-repo-lane-scale-matrix: $*" >&2
  exit 64
}

sha256_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

handle_signal() {
  local name=$1
  local number=$2
  local status=$((128 + number))
  trap - INT TERM HUP
  if [[ -n $active_inner_pid ]]; then
    local pgid=$active_inner_pid
    if [[ -f $active_inner_pgid_file ]]; then
      local recorded
      recorded=$(sed -n '1p' "$active_inner_pgid_file" 2>/dev/null || true)
      [[ $recorded =~ ^[1-9][0-9]*$ && $recorded == "$active_inner_pid" ]] && pgid=$recorded
    fi
    kill -s "$name" -- "-$pgid" 2>/dev/null || kill -s "$name" "$active_inner_pid" 2>/dev/null || true
    python3 - "$pgid" "$active_inner_pid" >/dev/null 2>&1 <<'PY' &
import os, signal, sys, time
pgid,pid=map(int,sys.argv[1:])
time.sleep(5)
try: os.killpg(pgid,signal.SIGKILL)
except ProcessLookupError:
    try: os.kill(pid,signal.SIGKILL)
    except ProcessLookupError: pass
PY
    local watchdog=$!
    wait "$active_inner_pid" 2>/dev/null || true
    kill "$watchdog" 2>/dev/null || true
    wait "$watchdog" 2>/dev/null || true
    if [[ -n $active_inner_run_dir && -d $active_inner_run_dir ]]; then
      printf '{"schema_version":1,"status":"FAIL","signal":"%s","exit_status":%s,"inner_pid":%s}\n' \
        "$name" "$status" "$active_inner_pid" > "$active_inner_run_dir/signal-failure.json"
    fi
  fi
  exit "$status"
}

trap 'handle_signal INT 2' INT
trap 'handle_signal TERM 15' TERM
trap 'handle_signal HUP 1' HUP

device_id() {
  python3 - "$1" <<'PY'
import os, sys
print(os.stat(sys.argv[1], follow_symlinks=False).st_dev)
PY
}

filesystem_type() {
  python3 - "$1" <<'PY'
import platform, re, subprocess, sys
path=sys.argv[1]
if platform.system() == "Darwin":
    try:
        # diskutil accepts mount points and device nodes, but not arbitrary paths.
        # Resolve the containing device without following PATH-supplied wrappers.
        rows=subprocess.check_output(["/bin/df","-P",path], text=True, errors="replace",
                                     stderr=subprocess.DEVNULL).splitlines()
        device=rows[-1].split()[0]
        value=subprocess.check_output(["/usr/sbin/diskutil","info",device], text=True,
                                      errors="replace",stderr=subprocess.DEVNULL)
        match=re.search(r"^\s*File System Personality:\s*(.+?)\s*$", value, re.M)
        if match:
            print(match.group(1).strip().lower())
            raise SystemExit
    except (OSError, subprocess.CalledProcessError):
        pass
print("unknown")
PY
}

canonical_directory() {
  python3 - "$1" <<'PY'
import os, stat, sys
path=os.path.abspath(os.path.normpath(sys.argv[1]))
metadata=os.lstat(path)
if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit(f"not a real directory: {path}")
if os.path.realpath(path) != path:
    raise SystemExit(f"directory path traverses a symlink: {path}")
print(path)
PY
}

canonical_executable() {
  python3 - "$1" <<'PY'
import os, stat, sys
path=os.path.abspath(os.path.normpath(sys.argv[1]))
metadata=os.lstat(path)
if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit(f"not a regular non-symlink file: {path}")
if os.path.realpath(path) != path:
    raise SystemExit(f"file path traverses a symlink: {path}")
if not os.access(path,os.X_OK):
    raise SystemExit(f"file is not executable: {path}")
print(path)
PY
}

secure_byte_copy_binary() {
  python3 - "$1" "$2" <<'PY'
import errno, os, stat, sys
source,destination=sys.argv[1:]
before=os.lstat(source)
if not stat.S_ISREG(before.st_mode) or stat.S_ISLNK(before.st_mode):
    raise SystemExit("binary source is not a regular non-symlink file")
source_flags=os.O_RDONLY | getattr(os,"O_NOFOLLOW",0)
destination_flags=os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os,"O_NOFOLLOW",0)
source_fd=os.open(source,source_flags)
try:
    opened=os.fstat(source_fd)
    fingerprint=lambda value:(value.st_dev,value.st_ino,value.st_mode,value.st_size,
                              value.st_mtime_ns,value.st_ctime_ns)
    if fingerprint(opened) != fingerprint(before):
        raise SystemExit("binary source raced before copy")
    destination_fd=os.open(destination,destination_flags,0o555)
    try:
        while True:
            chunk=os.read(source_fd,1024*1024)
            if not chunk: break
            view=memoryview(chunk)
            while view:
                written=os.write(destination_fd,view)
                if written <= 0: raise OSError(errno.EIO,"short binary write")
                view=view[written:]
        os.fchmod(destination_fd,0o555)
        os.fsync(destination_fd)
    finally:
        os.close(destination_fd)
    if fingerprint(os.fstat(source_fd)) != fingerprint(before) or fingerprint(os.lstat(source)) != fingerprint(before):
        raise SystemExit("binary source raced during copy")
finally:
    os.close(source_fd)
parent_fd=os.open(os.path.dirname(destination),os.O_RDONLY | getattr(os,"O_DIRECTORY",0))
try: os.fsync(parent_fd)
finally: os.close(parent_fd)
PY
}

assert_pinned_binary() {
  python3 - "$pinned_trail" <<'PY'
import os, stat, sys
metadata=os.lstat(sys.argv[1])
if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit("pinned binary is not a regular non-symlink file")
PY
  [[ $(sha256_file "$pinned_trail") == "$pinned_trail_sha" ]] ||
    die "pinned TRAIL_BIN changed"
}

normalized_absent_path() {
  python3 - "$1" <<'PY'
import os, stat, sys
path=os.path.abspath(os.path.normpath(sys.argv[1]))
if os.path.lexists(path):
    raise SystemExit(f"path already exists: {path}")
parent=os.path.dirname(path)
metadata=os.lstat(parent)
if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit(f"parent is not a real directory: {parent}")
if os.path.realpath(parent) != parent or os.path.realpath(path) != path:
    raise SystemExit(f"output path traverses a symlink: {path}")
print(path)
PY
}

paths_overlap() {
  python3 - "$1" "$2" <<'PY'
import os, sys
left,right=map(os.path.realpath,sys.argv[1:])
overlap=os.path.commonpath([left,right]) in {left,right}
raise SystemExit(0 if overlap else 1)
PY
}

require_ref_absent() {
  local ref=$1
  set +e
  git -C "$TRAIL_SCALE_REPO" show-ref --verify --quiet "$ref"
  local code=$?
  set -e
  case $code in
    0) die "final run ref already exists: $ref" ;;
    1) return 0 ;;
    *) die "git show-ref operational failure for $ref (exit $code)" ;;
  esac
}

capture_inventory() {
  local root=$1
  local mode=$2
  local output=$3
  python3 - "$root" "$mode" "$output" <<'PY'
import hashlib, json, os, stat, sys
root, mode, output=sys.argv[1:]
root=os.path.realpath(root)
rows=[]

def digest_file(path):
    digest=hashlib.sha256()
    with open(path,"rb",buffering=0) as stream:
        while True:
            chunk=stream.read(1024*1024)
            if not chunk: break
            digest.update(chunk)
    return digest.hexdigest()

def visit(directory):
    with os.scandir(directory) as stream:
        entries=sorted(stream, key=lambda entry: os.fsencode(entry.name))
    for entry in entries:
        path=entry.path
        relative=os.path.relpath(path,root).replace(os.sep,"/")
        first=relative.split("/",1)[0]
        if mode == "checkout" and first == ".git": continue
        metadata=os.lstat(path)
        permission=stat.S_IMODE(metadata.st_mode)
        row={"path":relative,"mode":permission}
        if stat.S_ISLNK(metadata.st_mode):
            target=os.readlink(path)
            row.update(type="symlink",size=len(os.fsencode(target)),
                       digest=hashlib.sha256(os.fsencode(target)).hexdigest())
        elif stat.S_ISDIR(metadata.st_mode):
            row.update(type="directory",size=None,digest=None)
            rows.append(row)
            visit(path)
            continue
        elif stat.S_ISREG(metadata.st_mode):
            row.update(type="regular",size=metadata.st_size,digest=digest_file(path))
        elif stat.S_ISFIFO(metadata.st_mode): row.update(type="fifo",size=None,digest=None)
        elif stat.S_ISSOCK(metadata.st_mode): row.update(type="socket",size=None,digest=None)
        elif stat.S_ISCHR(metadata.st_mode): row.update(type="char_device",size=None,digest=None)
        elif stat.S_ISBLK(metadata.st_mode): row.update(type="block_device",size=None,digest=None)
        else: row.update(type="other",size=None,digest=None)
        rows.append(row)

visit(root)
with open(output,"w",encoding="utf-8") as stream:
    json.dump({"schema_version":1,"root":root,"mode":mode,"entries":rows},stream,
              sort_keys=True,separators=(",",":"))
    stream.write("\n")
PY
}

capture_source_snapshot() {
  local root=$1
  local mode=$2
  local output=$3
  local inventory=$4
  GIT_OPTIONAL_LOCKS=0 python3 - "$root" "$mode" "$output" "$inventory" <<'PY'
import hashlib, json, os, stat, subprocess, sys
root,mode,output,inventory=sys.argv[1:]

def git(*args, raw=False):
    env=dict(os.environ,GIT_OPTIONAL_LOCKS="0")
    value=subprocess.check_output(["git","-C",root,*args],env=env)
    return value if raw else value.decode().strip()

head=git("rev-parse","HEAD")
symbolic=git("symbolic-ref","HEAD")
refs=git("for-each-ref","--format=%(refname)%00%(objectname)",raw=True).splitlines()
status_bytes=git("status","--porcelain=v2","-z","--untracked-files=all","--ignored=matching",raw=True)
index_path=git("rev-parse","--git-path","index")
if not os.path.isabs(index_path): index_path=os.path.join(root,index_path)
index_path=os.path.realpath(index_path)
metadata=os.lstat(index_path)
if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit("Git index is not a regular non-symlink file")
index_bytes=open(index_path,"rb").read()
value={
    "schema_version":1,
    "mode":mode,
    "head":head,
    "symbolic_head":symbolic,
    "refs":[line.decode() for line in refs],
    "status_sha256":hashlib.sha256(status_bytes).hexdigest(),
    "status_hex":status_bytes.hex(),
    "index":{"path":index_path,"mode":stat.S_IMODE(metadata.st_mode),
             "size":len(index_bytes),"sha256":hashlib.sha256(index_bytes).hexdigest()},
    "inventory":json.load(open(inventory,encoding="utf-8"))["entries"],
}
with open(output,"w",encoding="utf-8") as stream:
    json.dump(value,stream,sort_keys=True,separators=(",",":")); stream.write("\n")
PY
}

assert_source_unchanged() {
  local phase=$1
  local current=$TRAIL_SCALE_MATRIX_OUTPUT/source-current.json
  local inventory=$TRAIL_SCALE_MATRIX_OUTPUT/source-current-inventory.json
  capture_inventory "$TRAIL_SCALE_REPO" full "$inventory"
  capture_source_snapshot "$TRAIL_SCALE_REPO" full "$current" "$inventory"
  cmp -s "$TRAIL_SCALE_MATRIX_OUTPUT/source-baseline.json" "$current" ||
    die "source repository drifted during $phase"
}

validate_copy_target() {
  local copy=$1
  local count=$2
  python3 - "$copy" "$TRAIL_SCALE_MATRIX_OUTPUT" "$count" "$source_device" <<'PY'
import os, stat, sys
copy,output,count,device=sys.argv[1:]
metadata=os.lstat(copy)
if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit("copy is not a real directory")
real=os.path.realpath(copy); parent=os.path.realpath(output)
if os.path.dirname(real) != parent or not os.path.basename(real).startswith(f"copy-{count}."):
    raise SystemExit("copy path is outside the exact mktemp namespace")
if str(os.stat(real).st_dev) != device:
    raise SystemExit("copy device differs from source")
print(real)
PY
}

validate_and_remove_copied_trail() {
  local copy=$1
  python3 - "$copy" "$source_device" <<'PY'
import os, stat, sys
copy,device=sys.argv[1:]
copy=os.path.realpath(copy); target=os.path.join(copy,".trail")
metadata=os.lstat(target)
if not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit("copied .trail is not a real directory")
if os.path.realpath(target) != target or os.path.dirname(target) != copy:
    raise SystemExit("copied .trail target is not exact")
if str(os.stat(target).st_dev) != device:
    raise SystemExit("copied .trail device differs from source")
print(target)
PY
  local target=$copy/.trail
  case "$target" in "$TRAIL_SCALE_MATRIX_OUTPUT"/copy-*/.trail) ;; *) die "unsafe copied .trail target" ;; esac
  rm -rf -- "$target"
  [[ ! -e $target && ! -L $target ]] || die "copied .trail removal failed"
}

write_owner_file() {
  local copy=$1
  local evidence=$2
  local run_id=$3
  local owner=$copy/.trail/scale-disposable-owner.json
  python3 - "$owner" "$TRAIL_SCALE_REPO" "$copy" "$evidence" "$run_id" <<'PY'
import json, os, sys
owner,source,copy,output,run_id=sys.argv[1:]
value={"schema_version":1,"kind":"trail_scale_disposable_workspace",
       "canonical_repo":os.path.realpath(source),"disposable_repo":os.path.realpath(copy),
       "output":os.path.abspath(os.path.normpath(output)),"run_id":run_id}
with open(owner,"x",encoding="utf-8") as stream:
    json.dump(value,stream,sort_keys=True,separators=(",",":")); stream.write("\n")
os.chmod(owner,0o600)
PY
  [[ -f $owner && ! -L $owner ]] || die "disposable owner file is unsafe"
  printf '%s\n' "$owner"
}

validate_owner_file() {
  local owner=$1
  local copy=$2
  local evidence=$3
  local run_id=$4
  python3 - "$owner" "$TRAIL_SCALE_REPO" "$copy" "$evidence" "$run_id" <<'PY'
import json, os, stat, sys
owner,source,copy,output,run_id=sys.argv[1:]
expected_path=os.path.join(copy,".trail","scale-disposable-owner.json")
if owner != expected_path: raise SystemExit("owner binding path mismatch")
metadata=os.lstat(owner)
if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit("owner binding file is unsafe")
value=json.load(open(owner,encoding="utf-8"))
expected={"schema_version":1,"kind":"trail_scale_disposable_workspace",
          "canonical_repo":source,"disposable_repo":copy,
          "output":os.path.abspath(os.path.normpath(output)),"run_id":run_id}
if value != expected: raise SystemExit("owner binding content mismatch")
PY
}

validate_checker_pass() {
  local checker=$1
  local count=$2
  python3 - "$checker" "$count" <<'PY'
import json, sys
path,count=sys.argv[1],int(sys.argv[2])
with open(path,encoding="utf-8") as stream:
    lines=stream.read().splitlines()
if len(lines) != 1: raise SystemExit("checker.out must contain one JSON line")
value=json.loads(lines[0])
if not isinstance(value,dict) or value.get("status") != "PASS" or value.get("lanes") != count:
    raise SystemExit("inner checker did not prove the requested PASS")
PY
}

validate_clone_manifest() {
  local manifest=$1
  local copy=$2
  python3 - "$manifest" "$TRAIL_SCALE_REPO" "$copy" "$source_device" <<'PY'
import json, os, stat, sys
path,source,destination,device=sys.argv[1:]
metadata=os.lstat(path)
if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit("clone manifest is not a regular non-symlink file")
value=json.load(open(path,encoding="utf-8"))
if value.get("schema_version") != 1 or value.get("status") != "PASS":
    raise SystemExit("clone manifest does not record PASS")
if value.get("clone_api") != "clonefile(2)" or value.get("byte_copy_fallback") is not False:
    raise SystemExit("clone manifest does not prove clonefile-only operation")
if value.get("source") != source or value.get("destination") != destination:
    raise SystemExit("clone manifest path binding mismatch")
if str(value.get("source_device")) != device or str(value.get("destination_device")) != device:
    raise SystemExit("clone manifest device binding mismatch")
counters=value.get("counters")
if not isinstance(counters,dict): raise SystemExit("clone manifest counters missing")
attempted=counters.get("clonefile_calls_attempted")
succeeded=counters.get("clonefile_calls_succeeded")
hardlinks=counters.get("hardlinks_created")
regular=counters.get("regular_paths")
if not all(isinstance(item,int) and item >= 0 for item in [attempted,succeeded,hardlinks,regular]):
    raise SystemExit("clone manifest counters invalid")
if attempted != succeeded or regular != succeeded + hardlinks:
    raise SystemExit("clone manifest regular-file accounting mismatch")
if counters.get("byte_copy_calls") != 0 or counters.get("special_entries_rejected") != 0:
    raise SystemExit("clone manifest records fallback or special entries")
calls=value.get("clonefile_calls")
if not isinstance(calls,list) or len(calls) != attempted:
    raise SystemExit("clone manifest call list incomplete")
for call in calls:
    if call.get("success") is not True:
        raise SystemExit("clone manifest contains a failed call")
    if str(call.get("source_device")) != device or str(call.get("destination_device")) != device:
        raise SystemExit("clone call device mismatch")
    if call.get("size") != call.get("destination_size"):
        raise SystemExit("clone call size mismatch")
for left,right in [("source_tree_sha256","destination_tree_sha256"),
                   ("source_inventory_sha256","destination_inventory_sha256")]:
    if not isinstance(value.get(left),str) or value[left] != value.get(right):
        raise SystemExit(f"clone manifest digest mismatch: {left}")
PY
}

run_inner_harness() {
  local count=$1
  local copy=$2
  local evidence=$3
  local run_id=$4
  local git_ref=$5
  local owner=$6
  local run_dir=$7
  local pgid_file=$run_dir/inner.pgid
  local completion_file=$run_dir/inner.supervisor-complete
  assert_pinned_binary
  validate_owner_file "$owner" "$copy" "$evidence" "$run_id" || die "owner binding rejected before $count-lane inner run"
  set +e
  TRAIL_BIN=$pinned_trail \
  TRAIL_SCALE_REPO=$copy \
  TRAIL_SCALE_OUTPUT=$evidence \
  TRAIL_SCALE_RUN_ID=$run_id \
  TRAIL_SCALE_GIT_REF=$git_ref \
  TRAIL_SCALE_LANES=$count \
  TRAIL_SCALE_CONCURRENCY=$count \
  TRAIL_SCALE_FILES_PER_LANE=$TRAIL_SCALE_FILES_PER_LANE \
  TRAIL_SCALE_FAULT_PHASE=$TRAIL_SCALE_FAULT_PHASE \
  TRAIL_SCALE_LATENCY_CEILING_SECONDS=$TRAIL_SCALE_LATENCY_CEILING_SECONDS \
  TRAIL_SCALE_EXPECTED_BINARY_SHA256=$pinned_trail_sha \
  TRAIL_SCALE_EXPECTED_SOURCE_COMMIT=$trail_source_commit \
  TRAIL_SCALE_FAULT_DRIVER=$fault_driver \
  TRAIL_SCALE_EXPECTED_FAULT_DRIVER_SHA256=$fault_driver_sha \
  TRAIL_SCALE_DISPOSABLE_WORKSPACE=1 \
  TRAIL_SCALE_DISPOSABLE_OWNER_FILE=$owner \
  python3 - "$pgid_file" "$completion_file" "$TRAIL_SCALE_INNER_HARNESS" >"$run_dir/inner.stdout" 2>"$run_dir/inner.stderr" <<'PY' &
import os, signal, subprocess, sys
ready,completion,program=sys.argv[1:]
os.setpgid(0,0)
descriptor=os.open(ready,os.O_WRONLY|os.O_CREAT|os.O_EXCL|getattr(os,"O_NOFOLLOW",0),0o600)
try:
    os.write(descriptor,(str(os.getpid())+"\n").encode())
    os.fsync(descriptor)
finally:
    os.close(descriptor)
def restore_signals():
    for number in (signal.SIGINT,signal.SIGTERM,signal.SIGHUP):
        signal.signal(number,signal.SIG_DFL)
child=subprocess.Popen([program],env=os.environ,preexec_fn=restore_signals)
for number in (signal.SIGINT,signal.SIGTERM,signal.SIGHUP):
    signal.signal(number,signal.SIG_IGN)
code=child.wait()
status=code if code >= 0 else 128-code
descriptor=os.open(completion,os.O_WRONLY|os.O_CREAT|os.O_EXCL|getattr(os,"O_NOFOLLOW",0),0o600)
try:
    os.write(descriptor,(str(status)+"\n").encode())
    os.fsync(descriptor)
finally:
    os.close(descriptor)
raise SystemExit(status)
PY
  active_inner_pid=$!
  active_inner_run_dir=$run_dir
  active_inner_pgid_file=$pgid_file
  while [[ ! -f $completion_file ]]; do
    state=$(ps -o stat= -p "$active_inner_pid" 2>/dev/null | tr -d '[:space:]')
    [[ -z $state || $state == Z* ]] && break
    /bin/sleep 0.1
  done
  wait "$active_inner_pid"
  local code=$?
  active_inner_pid=
  active_inner_run_dir=
  active_inner_pgid_file=
  set -e
  printf '%s\n' "$code" > "$run_dir/inner.exit-code"
  assert_source_unchanged "$count-lane inner run"
  validate_owner_file "$owner" "$copy" "$evidence" "$run_id" || die "owner binding rejected after $count-lane inner run"
  (( code == 0 )) || die "inner harness failed for $count lanes with exit $code; evidence retained at $run_dir"
  [[ -f $evidence/checker.out ]] || die "inner harness omitted checker.out for $count lanes"
  validate_checker_pass "$evidence/checker.out" "$count" || die "inner checker rejected $count-lane evidence"
}

revalidate_run_proof() {
  local count=$1
  local copy=$2
  local evidence=$3
  local run_id=$4
  local git_ref=$5
  local baseline=$6
  local run_dir=$7
  local proof=$run_dir/proof.json
  python3 - "$proof" "$count" "$run_id" "$copy" "$evidence" "$git_ref" "$baseline" \
    "$run_dir/final.bundle" "$pinned_trail_sha" <<'PY'
import hashlib, json, os, stat, sys
proof_path,count,run_id,copy,evidence,ref,baseline,bundle,binary_sha=sys.argv[1:]
expected_keys={"schema_version","lanes","run_id","copy","evidence","ref","baseline",
               "commit","tree","bundle","bundle_sha256","checker_sha256","binary_sha256"}
metadata=os.lstat(proof_path)
if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
    raise SystemExit("proof is not a regular non-symlink file")
value=json.load(open(proof_path,encoding="utf-8"))
if set(value) != expected_keys: raise SystemExit("proof schema is not exact")
expected={"schema_version":1,"lanes":int(count),"run_id":run_id,"copy":copy,
          "evidence":evidence,"ref":ref,"baseline":baseline,"bundle":bundle,
          "binary_sha256":binary_sha}
for key,item in expected.items():
    if value.get(key) != item: raise SystemExit(f"proof binding mismatch: {key}")
def digest(path):
    metadata=os.lstat(path)
    if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
        raise SystemExit(f"proof artifact is unsafe: {path}")
    result=hashlib.sha256()
    with open(path,"rb") as stream:
        while True:
            chunk=stream.read(1024*1024)
            if not chunk: break
            result.update(chunk)
    return result.hexdigest()
if digest(bundle) != value["bundle_sha256"]: raise SystemExit("bundle hash mismatch")
checker=os.path.join(evidence,"checker.out")
if digest(checker) != value["checker_sha256"]: raise SystemExit("checker hash mismatch")
PY
  [[ $? -eq 0 ]] || return 1
  local identities
  identities=$(python3 - "$proof" <<'PY'
import json,sys
value=json.load(open(sys.argv[1],encoding="utf-8")); print(value["commit"]); print(value["tree"])
PY
) || return 1
  local commit
  local tree
  commit=$(printf '%s\n' "$identities" | sed -n '1p')
  tree=$(printf '%s\n' "$identities" | sed -n '2p')
  [[ $(git -C "$copy" rev-parse "$git_ref^{commit}") == "$commit" ]] || return 1
  [[ $(git -C "$copy" rev-parse "$commit^{tree}") == "$tree" ]] || return 1
  git -C "$copy" merge-base --is-ancestor "$baseline" "$commit" || return 1
  validate_checker_pass "$evidence/checker.out" "$count" || return 1
  git -C "$copy" bundle verify "$run_dir/final.bundle" >/dev/null 2>&1 || return 1
  local heads
  heads=$(git -C "$copy" bundle list-heads "$run_dir/final.bundle") || return 1
  [[ $heads == "$commit $git_ref" ]] || return 1
  assert_pinned_binary
}

create_run_proof() {
  local count=$1
  local copy=$2
  local evidence=$3
  local run_id=$4
  local git_ref=$5
  local baseline=$6
  local run_dir=$7
  local commit
  local tree
  local bundle=$run_dir/final.bundle
  commit=$(git -C "$copy" rev-parse "$git_ref^{commit}") || die "missing dedicated ref after $count-lane PASS"
  git -C "$copy" merge-base --is-ancestor "$baseline" "$commit" || die "$count-lane commit is not based on copied HEAD"
  tree=$(git -C "$copy" rev-parse "$commit^{tree}")
  git -C "$copy" bundle create "$bundle" "$git_ref" >/dev/null
  git -C "$copy" bundle verify "$bundle" >"$run_dir/bundle.verify" 2>&1 || die "$count-lane bundle verification failed"
  python3 - "$run_dir/proof.json" "$count" "$run_id" "$copy" "$evidence" "$git_ref" \
    "$baseline" "$commit" "$tree" "$bundle" "$(sha256_file "$bundle")" \
    "$(sha256_file "$evidence/checker.out")" "$pinned_trail_sha" <<'PY'
import json, sys
(path,count,run_id,copy,evidence,ref,baseline,commit,tree,bundle,bundle_sha,checker_sha,binary_sha)=sys.argv[1:]
value={"schema_version":1,"lanes":int(count),"run_id":run_id,"copy":copy,
       "evidence":evidence,"ref":ref,"baseline":baseline,"commit":commit,"tree":tree,
       "bundle":bundle,"bundle_sha256":bundle_sha,"checker_sha256":checker_sha,
       "binary_sha256":binary_sha}
with open(path,"w",encoding="utf-8") as stream:
    json.dump(value,stream,sort_keys=True,separators=(",",":")); stream.write("\n")
PY
}

proof_field() {
  python3 - "$1" "$2" <<'PY'
import json, sys
print(json.load(open(sys.argv[1],encoding="utf-8"))[sys.argv[2]])
PY
}

verify_postpublication_source() {
  local proof64=$1
  local proof128=$2
  local worktree_inventory=$TRAIL_SCALE_MATRIX_OUTPUT/source-postpublish-checkout.json
  local source_snapshot=$TRAIL_SCALE_MATRIX_OUTPUT/source-postpublish.json
  capture_inventory "$TRAIL_SCALE_REPO" checkout "$worktree_inventory"
  capture_source_snapshot "$TRAIL_SCALE_REPO" checkout "$source_snapshot" "$worktree_inventory"
  python3 - "$TRAIL_SCALE_MATRIX_OUTPUT/source-checkout-baseline.json" "$source_snapshot" \
    "$proof64" "$proof128" <<'PY'
import json, sys
before=json.load(open(sys.argv[1],encoding="utf-8")); after=json.load(open(sys.argv[2],encoding="utf-8"))
proofs=[json.load(open(path,encoding="utf-8")) for path in sys.argv[3:]]
for key in ["head","symbolic_head","status_sha256","status_hex","index","inventory"]:
    if before[key] != after[key]: raise SystemExit(f"source checkout drifted after publication: {key}")
expected={proof["ref"]+"\0"+proof["commit"] for proof in proofs}
before_refs=set(before["refs"]); after_refs=set(after["refs"])
if not before_refs.issubset(after_refs): raise SystemExit("an original source ref changed or disappeared")
if after_refs-before_refs != expected: raise SystemExit("publication changed refs outside the exact run refs")
PY
}

[[ $# -eq 0 ]] || die "usage: $0"
[[ -n $TRAIL_SCALE_REPO ]] || die "TRAIL_SCALE_REPO is required"
[[ $TRAIL_SCALE_REPO == /* ]] || die "TRAIL_SCALE_REPO must be absolute"
[[ -n $TRAIL_BIN ]] || die "TRAIL_BIN is required"
[[ $TRAIL_BIN == /* ]] || die "TRAIL_BIN must be absolute"
[[ -n $TRAIL_SCALE_MATRIX_OUTPUT ]] || die "TRAIL_SCALE_MATRIX_OUTPUT is required"
[[ $TRAIL_SCALE_MATRIX_OUTPUT == /* ]] || die "TRAIL_SCALE_MATRIX_OUTPUT must be absolute"
[[ $TRAIL_SCALE_INNER_HARNESS == /* ]] || die "TRAIL_SCALE_INNER_HARNESS must be absolute"
case "$TRAIL_SCALE_MATRIX_PUBLISH" in 0|1) ;; *) die "TRAIL_SCALE_MATRIX_PUBLISH must be 0 or 1" ;; esac
[[ $TRAIL_SCALE_FILES_PER_LANE =~ ^[1-9][0-9]*$ ]] || die "TRAIL_SCALE_FILES_PER_LANE must be positive"

TRAIL_SCALE_REPO=$(canonical_directory "$TRAIL_SCALE_REPO") || die "TRAIL_SCALE_REPO must be a real directory"
[[ -d $TRAIL_SCALE_REPO/.git && ! -L $TRAIL_SCALE_REPO/.git ]] || die "TRAIL_SCALE_REPO must have an independent real .git directory"
git -C "$TRAIL_SCALE_REPO" rev-parse --is-inside-work-tree >/dev/null 2>&1 || die "TRAIL_SCALE_REPO must be a Git worktree"
GIT_OPTIONAL_LOCKS=0 git -C "$TRAIL_SCALE_REPO" diff --quiet -- || die "source tracked worktree must be clean"
GIT_OPTIONAL_LOCKS=0 git -C "$TRAIL_SCALE_REPO" diff --cached --quiet -- || die "source Git index must be clean"
git -C "$TRAIL_SCALE_REPO" symbolic-ref -q HEAD >/dev/null || die "source HEAD must be symbolic"

TRAIL_BIN=$(canonical_executable "$TRAIL_BIN") || die "TRAIL_BIN must be an absolute executable regular file without symlink traversal"
TRAIL_SCALE_INNER_HARNESS=$(canonical_executable "$TRAIL_SCALE_INNER_HARNESS") ||
  die "TRAIL_SCALE_INNER_HARNESS must be an executable regular file without symlink traversal"
APFS_CLONE_TREE_HELPER=$(canonical_executable "$APFS_CLONE_TREE_HELPER") ||
  die "APFS clone-tree helper must be an executable regular file without symlink traversal"
fault_driver=${TRAIL_SCALE_FAULT_DRIVER:-$TRAIL_SCALE_INNER_HARNESS}
[[ $fault_driver == /* ]] || die "TRAIL_SCALE_FAULT_DRIVER must be absolute"
fault_driver=$(canonical_executable "$fault_driver") ||
  die "TRAIL_SCALE_FAULT_DRIVER must be an executable regular file without symlink traversal"

TRAIL_SCALE_MATRIX_OUTPUT=$(normalized_absent_path "$TRAIL_SCALE_MATRIX_OUTPUT") || die "TRAIL_SCALE_MATRIX_OUTPUT must be an absent path with a real parent"
output_parent=$(dirname -- "$TRAIL_SCALE_MATRIX_OUTPUT")
paths_overlap "$TRAIL_SCALE_REPO" "$TRAIL_SCALE_MATRIX_OUTPUT" && die "source and matrix output must not overlap"

source_device=$(device_id "$TRAIL_SCALE_REPO")
output_device=$(device_id "$output_parent")
[[ $source_device == "$output_device" ]] || die "source and matrix output must be on the same device"
[[ $(filesystem_type "$TRAIL_SCALE_REPO") == apfs ]] || die "TRAIL_SCALE_REPO must be on APFS"
[[ $(filesystem_type "$output_parent") == apfs ]] || die "TRAIL_SCALE_MATRIX_OUTPUT parent must be on APFS"

if [[ -z $TRAIL_SCALE_MATRIX_RUN_ID ]]; then
  TRAIL_SCALE_MATRIX_RUN_ID=$(python3 - <<'PY'
import datetime, os, secrets
print("matrix-"+datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")+f"-{os.getpid()}-"+secrets.token_hex(6))
PY
)
fi
[[ $TRAIL_SCALE_MATRIX_RUN_ID =~ ^[A-Za-z0-9._-]+$ ]] || die "TRAIL_SCALE_MATRIX_RUN_ID contains unsafe characters"
ref64=refs/heads/codex/trail-scale-$TRAIL_SCALE_MATRIX_RUN_ID-64
ref128=refs/heads/codex/trail-scale-$TRAIL_SCALE_MATRIX_RUN_ID-128
[[ $ref64 != "$ref128" ]] || die "matrix refs are not unique"
git check-ref-format "$ref64" >/dev/null 2>&1 || die "TRAIL_SCALE_MATRIX_RUN_ID does not produce a valid Git ref"
git check-ref-format "$ref128" >/dev/null 2>&1 || die "TRAIL_SCALE_MATRIX_RUN_ID does not produce a valid Git ref"
require_ref_absent "$ref64"
require_ref_absent "$ref128"

mkdir -- "$TRAIL_SCALE_MATRIX_OUTPUT"
mkdir -- "$TRAIL_SCALE_MATRIX_OUTPUT/pinned" "$TRAIL_SCALE_MATRIX_OUTPUT/runs"
capture_inventory "$TRAIL_SCALE_REPO" full "$TRAIL_SCALE_MATRIX_OUTPUT/source-baseline-inventory.json"
capture_source_snapshot "$TRAIL_SCALE_REPO" full "$TRAIL_SCALE_MATRIX_OUTPUT/source-baseline.json" \
  "$TRAIL_SCALE_MATRIX_OUTPUT/source-baseline-inventory.json"
capture_inventory "$TRAIL_SCALE_REPO" checkout "$TRAIL_SCALE_MATRIX_OUTPUT/source-checkout-baseline-inventory.json"
capture_source_snapshot "$TRAIL_SCALE_REPO" checkout "$TRAIL_SCALE_MATRIX_OUTPUT/source-checkout-baseline.json" \
  "$TRAIL_SCALE_MATRIX_OUTPUT/source-checkout-baseline-inventory.json"

pinned_trail=$TRAIL_SCALE_MATRIX_OUTPUT/pinned/trail
secure_byte_copy_binary "$TRAIL_BIN" "$pinned_trail" || die "secure pinned TRAIL_BIN byte copy failed"
pinned_trail_sha=$(sha256_file "$pinned_trail")
[[ $pinned_trail_sha == "$(sha256_file "$TRAIL_BIN")" ]] || die "pinned TRAIL_BIN digest mismatch"
"$pinned_trail" --version > "$TRAIL_SCALE_MATRIX_OUTPUT/pinned/version.txt" 2>&1 || die "pinned TRAIL_BIN --version failed"
"$pinned_trail" init --help > "$TRAIL_SCALE_MATRIX_OUTPUT/pinned/init-help.txt" 2>&1 || die "pinned TRAIL_BIN init --help failed"
grep -q -- '--from-git' "$TRAIL_SCALE_MATRIX_OUTPUT/pinned/init-help.txt" || die "pinned TRAIL_BIN lacks init --from-git"
chmod 0555 "$TRAIL_SCALE_MATRIX_OUTPUT/pinned"

trail_source_commit=$(git -C "$PROJECT_ROOT" rev-parse HEAD)
fault_driver_sha=$(sha256_file "$fault_driver")
assert_source_unchanged "matrix preparation"

for count in 64 128; do
  run_dir=$TRAIL_SCALE_MATRIX_OUTPUT/runs/$count
  mkdir -- "$run_dir"
  copy=$(mktemp -d "$TRAIL_SCALE_MATRIX_OUTPUT/copy-$count.XXXXXX")
  copy=$(validate_copy_target "$copy" "$count") || die "mktemp returned an unsafe $count-lane copy path"
  [[ $(filesystem_type "$copy") == apfs ]] || die "$count-lane copy is not on APFS"
  clone_manifest=$run_dir/clone-manifest.json
  python3 "$APFS_CLONE_TREE_HELPER" "$TRAIL_SCALE_REPO" "$copy" "$clone_manifest" ||
    die "clonefile-only tree clone failed for $count lanes; evidence retained at $run_dir"
  validate_clone_manifest "$clone_manifest" "$copy" || die "clone manifest rejected for $count lanes"
  capture_inventory "$copy" full "$run_dir/copy-before-trail-removal.json"
  python3 - "$TRAIL_SCALE_MATRIX_OUTPUT/source-baseline-inventory.json" "$run_dir/copy-before-trail-removal.json" <<'PY' || die "APFS copy inventory mismatch"
import json, sys
left=json.load(open(sys.argv[1],encoding="utf-8")); right=json.load(open(sys.argv[2],encoding="utf-8"))
if left["entries"] != right["entries"]: raise SystemExit("copy did not preserve every source entry")
PY
  validate_and_remove_copied_trail "$copy"
  assert_pinned_binary
  "$pinned_trail" --workspace "$copy" --json init --from-git > "$run_dir/init.json" 2> "$run_dir/init.stderr" ||
    die "copy-local trail init --from-git failed for $count lanes"
  assert_pinned_binary
  evidence=$run_dir/evidence
  run_id=$TRAIL_SCALE_MATRIX_RUN_ID-$count
  if [[ $count == 64 ]]; then git_ref=$ref64; else git_ref=$ref128; fi
  owner=$(write_owner_file "$copy" "$evidence" "$run_id")
  validate_owner_file "$owner" "$copy" "$evidence" "$run_id" || die "owner binding rejected after creation"
  baseline=$(git -C "$copy" rev-parse HEAD)
  if [[ $count == 64 ]]; then
    copy64=$copy; evidence64=$evidence; run_id64=$run_id; baseline64=$baseline
  else
    copy128=$copy; evidence128=$evidence; run_id128=$run_id; baseline128=$baseline
  fi
  assert_source_unchanged "$count-lane copy preparation"
  run_inner_harness "$count" "$copy" "$evidence" "$run_id" "$git_ref" "$owner" "$run_dir"
  create_run_proof "$count" "$copy" "$evidence" "$run_id" "$git_ref" "$baseline" "$run_dir"
  [[ $(sha256_file "$pinned_trail") == "$pinned_trail_sha" ]] || die "pinned TRAIL_BIN changed during $count-lane run"
  assert_source_unchanged "$count-lane run"
done

proof64=$TRAIL_SCALE_MATRIX_OUTPUT/runs/64/proof.json
proof128=$TRAIL_SCALE_MATRIX_OUTPUT/runs/128/proof.json
assert_source_unchanged "pre-publication"

if [[ $TRAIL_SCALE_MATRIX_PUBLISH == 1 ]]; then
  revalidate_run_proof 64 "$copy64" "$evidence64" "$run_id64" "$ref64" "$baseline64" \
    "$TRAIL_SCALE_MATRIX_OUTPUT/runs/64" || die "proof revalidation failed for 64 lanes"
  revalidate_run_proof 128 "$copy128" "$evidence128" "$run_id128" "$ref128" "$baseline128" \
    "$TRAIL_SCALE_MATRIX_OUTPUT/runs/128" || die "proof revalidation failed for 128 lanes"
  assert_source_unchanged "proof revalidation"
  require_ref_absent "$ref64"
  require_ref_absent "$ref128"
  commit64=$(proof_field "$proof64" commit)
  commit128=$(proof_field "$proof128" commit)
  bundle64=$(proof_field "$proof64" bundle)
  bundle128=$(proof_field "$proof128" bundle)
  git -C "$TRAIL_SCALE_REPO" fetch --no-tags --no-write-fetch-head "$bundle64" "$ref64" >/dev/null
  git -C "$TRAIL_SCALE_REPO" fetch --no-tags --no-write-fetch-head "$bundle128" "$ref128" >/dev/null
  [[ $(git -C "$TRAIL_SCALE_REPO" rev-parse "$commit64^{tree}") == "$(proof_field "$proof64" tree)" ]] || die "imported 64-lane tree mismatch"
  [[ $(git -C "$TRAIL_SCALE_REPO" rev-parse "$commit128^{tree}") == "$(proof_field "$proof128" tree)" ]] || die "imported 128-lane tree mismatch"
  {
    echo start
    echo "create $ref64 $commit64"
    echo "create $ref128 $commit128"
    echo prepare
    echo commit
  } | git -C "$TRAIL_SCALE_REPO" update-ref --stdin >/dev/null || die "atomic absent-only publication CAS failed"
  verify_postpublication_source "$proof64" "$proof128" || die "source drifted outside exact published refs"
fi

python3 - "$TRAIL_SCALE_MATRIX_OUTPUT/matrix-summary.json" "$TRAIL_SCALE_MATRIX_RUN_ID" \
  "$TRAIL_SCALE_MATRIX_PUBLISH" "$TRAIL_SCALE_REPO" "$proof64" "$proof128" <<'PY'
import json, sys
path,run_id,published,source,*proof_paths=sys.argv[1:]
proofs=[json.load(open(item,encoding="utf-8")) for item in proof_paths]
value={"schema_version":1,"status":"PASS","run_id":run_id,"published":published=="1",
       "source":source,"runs":proofs}
with open(path,"w",encoding="utf-8") as stream:
    json.dump(value,stream,sort_keys=True,separators=(",",":")); stream.write("\n")
PY

echo "disposable real-repository lane scale matrix PASS: $TRAIL_SCALE_MATRIX_OUTPUT"
