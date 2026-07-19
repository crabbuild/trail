#!/usr/bin/env bash
# Run the 64/128 real-repository scale matrix only in independent APFS copies.
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)

TRAIL_BIN=${TRAIL_BIN:-}
TRAIL_SCALE_REPO=${TRAIL_SCALE_REPO:-}
TRAIL_SCALE_MATRIX_OUTPUT=${TRAIL_SCALE_MATRIX_OUTPUT:-}
TRAIL_SCALE_INNER_HARNESS=${TRAIL_SCALE_INNER_HARNESS:-$SCRIPT_DIR/verify-real-repo-lane-scale.sh}
TRAIL_SCALE_MATRIX_RUN_ID=${TRAIL_SCALE_MATRIX_RUN_ID:-}
TRAIL_SCALE_MATRIX_PUBLISH=${TRAIL_SCALE_MATRIX_PUBLISH:-0}
TRAIL_SCALE_FILES_PER_LANE=${TRAIL_SCALE_FILES_PER_LANE:-50}
TRAIL_SCALE_FAULT_PHASE=${TRAIL_SCALE_FAULT_PHASE:-all}
TRAIL_SCALE_LATENCY_CEILING_SECONDS=${TRAIL_SCALE_LATENCY_CEILING_SECONDS:-120}

die() {
  echo "verify-real-repo-lane-scale-matrix: $*" >&2
  exit 64
}

sha256_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

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
        if mode in {"source","worktree"} and first == ".trail": continue
        if mode == "worktree" and first == ".git": continue
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
  capture_inventory "$TRAIL_SCALE_REPO" source "$inventory"
  capture_source_snapshot "$TRAIL_SCALE_REPO" source "$current" "$inventory"
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

run_inner_harness() {
  local count=$1
  local copy=$2
  local evidence=$3
  local run_id=$4
  local git_ref=$5
  local owner=$6
  local run_dir=$7
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
  "$TRAIL_SCALE_INNER_HARNESS" >"$run_dir/inner.stdout" 2>"$run_dir/inner.stderr"
  local code=$?
  set -e
  printf '%s\n' "$code" > "$run_dir/inner.exit-code"
  assert_source_unchanged "$count-lane inner run"
  (( code == 0 )) || die "inner harness failed for $count lanes with exit $code; evidence retained at $run_dir"
  [[ -f $evidence/checker.out ]] || die "inner harness omitted checker.out for $count lanes"
  validate_checker_pass "$evidence/checker.out" "$count" || die "inner checker rejected $count-lane evidence"
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
  local worktree_inventory=$TRAIL_SCALE_MATRIX_OUTPUT/source-postpublish-worktree.json
  local source_snapshot=$TRAIL_SCALE_MATRIX_OUTPUT/source-postpublish.json
  capture_inventory "$TRAIL_SCALE_REPO" worktree "$worktree_inventory"
  capture_source_snapshot "$TRAIL_SCALE_REPO" worktree "$source_snapshot" "$worktree_inventory"
  python3 - "$TRAIL_SCALE_MATRIX_OUTPUT/source-worktree-baseline.json" "$source_snapshot" \
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
if [[ $TRAIL_SCALE_MATRIX_PUBLISH == 1 ]]; then
  git -C "$TRAIL_SCALE_REPO" show-ref --verify --quiet "$ref64" && die "64-lane publication ref already exists"
  git -C "$TRAIL_SCALE_REPO" show-ref --verify --quiet "$ref128" && die "128-lane publication ref already exists"
fi

mkdir -- "$TRAIL_SCALE_MATRIX_OUTPUT"
mkdir -- "$TRAIL_SCALE_MATRIX_OUTPUT/pinned" "$TRAIL_SCALE_MATRIX_OUTPUT/runs"
capture_inventory "$TRAIL_SCALE_REPO" source "$TRAIL_SCALE_MATRIX_OUTPUT/source-baseline-inventory.json"
capture_source_snapshot "$TRAIL_SCALE_REPO" source "$TRAIL_SCALE_MATRIX_OUTPUT/source-baseline.json" \
  "$TRAIL_SCALE_MATRIX_OUTPUT/source-baseline-inventory.json"
capture_inventory "$TRAIL_SCALE_REPO" worktree "$TRAIL_SCALE_MATRIX_OUTPUT/source-worktree-baseline-inventory.json"
capture_source_snapshot "$TRAIL_SCALE_REPO" worktree "$TRAIL_SCALE_MATRIX_OUTPUT/source-worktree-baseline.json" \
  "$TRAIL_SCALE_MATRIX_OUTPUT/source-worktree-baseline-inventory.json"
capture_inventory "$TRAIL_SCALE_REPO" full "$TRAIL_SCALE_MATRIX_OUTPUT/source-full-inventory.json"

pinned_trail=$TRAIL_SCALE_MATRIX_OUTPUT/pinned/trail
/bin/cp -cp "$TRAIL_BIN" "$pinned_trail"
chmod 0555 "$pinned_trail"
pinned_trail_sha=$(sha256_file "$pinned_trail")
[[ $pinned_trail_sha == "$(sha256_file "$TRAIL_BIN")" ]] || die "pinned TRAIL_BIN digest mismatch"
"$pinned_trail" --version > "$TRAIL_SCALE_MATRIX_OUTPUT/pinned/version.txt" 2>&1 || die "pinned TRAIL_BIN --version failed"
"$pinned_trail" init --help > "$TRAIL_SCALE_MATRIX_OUTPUT/pinned/init-help.txt" 2>&1 || die "pinned TRAIL_BIN init --help failed"
grep -q -- '--from-git' "$TRAIL_SCALE_MATRIX_OUTPUT/pinned/init-help.txt" || die "pinned TRAIL_BIN lacks init --from-git"

trail_source_commit=$(git -C "$PROJECT_ROOT" rev-parse HEAD)
fault_driver_sha=$(sha256_file "$fault_driver")
assert_source_unchanged "matrix preparation"

for count in 64 128; do
  run_dir=$TRAIL_SCALE_MATRIX_OUTPUT/runs/$count
  mkdir -- "$run_dir"
  copy=$(mktemp -d "$TRAIL_SCALE_MATRIX_OUTPUT/copy-$count.XXXXXX")
  copy=$(validate_copy_target "$copy" "$count") || die "mktemp returned an unsafe $count-lane copy path"
  [[ $(filesystem_type "$copy") == apfs ]] || die "$count-lane copy is not on APFS"
  /bin/cp -cRp "$TRAIL_SCALE_REPO/." "$copy/"
  capture_inventory "$copy" full "$run_dir/copy-before-trail-removal.json"
  python3 - "$TRAIL_SCALE_MATRIX_OUTPUT/source-full-inventory.json" "$run_dir/copy-before-trail-removal.json" <<'PY' || die "APFS copy inventory mismatch"
import json, sys
left=json.load(open(sys.argv[1],encoding="utf-8")); right=json.load(open(sys.argv[2],encoding="utf-8"))
if left["entries"] != right["entries"]: raise SystemExit("copy did not preserve every source entry")
PY
  validate_and_remove_copied_trail "$copy"
  "$pinned_trail" --workspace "$copy" --json init --from-git > "$run_dir/init.json" 2> "$run_dir/init.stderr" ||
    die "copy-local trail init --from-git failed for $count lanes"
  evidence=$run_dir/evidence
  run_id=$TRAIL_SCALE_MATRIX_RUN_ID-$count
  if [[ $count == 64 ]]; then git_ref=$ref64; else git_ref=$ref128; fi
  owner=$(write_owner_file "$copy" "$evidence" "$run_id")
  baseline=$(git -C "$copy" rev-parse HEAD)
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
  git -C "$TRAIL_SCALE_REPO" show-ref --verify --quiet "$ref64" && die "64-lane publication ref appeared before CAS"
  git -C "$TRAIL_SCALE_REPO" show-ref --verify --quiet "$ref128" && die "128-lane publication ref appeared before CAS"
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
