export interface TaskOverlapInput {
  id: string;
  lane: string;
  title: string;
  status: string;
  provider?: string | undefined;
  changedPaths: string[];
}

export interface TaskOverlap {
  taskId: string;
  lane: string;
  title: string;
  status: string;
  provider?: string | undefined;
  sharedPaths: string[];
  changedPaths: number;
}

const INACTIVE_STATUSES = new Set(["empty", "applied", "archived", "removed", "deleted"]);
const MAX_OVERLAPS = 8;
const MAX_SHARED_PATHS = 12;

export function findTaskOverlaps(current: TaskOverlapInput | undefined, tasks: TaskOverlapInput[]): TaskOverlap[] {
  if (!current) {
    return [];
  }
  const currentPaths = new Set(current.changedPaths.map(normalizePath).filter(Boolean));
  if (!currentPaths.size) {
    return [];
  }

  const overlaps: TaskOverlap[] = [];
  for (const task of tasks) {
    if (sameTask(current, task) || isInactive(task.status)) {
      continue;
    }
    const sharedPaths = uniqueStrings(
      task.changedPaths
        .map((path) => ({ original: path, normalized: normalizePath(path) }))
        .filter((path) => path.normalized && currentPaths.has(path.normalized))
        .map((path) => path.original)
    ).slice(0, MAX_SHARED_PATHS);
    if (!sharedPaths.length) {
      continue;
    }
    overlaps.push({
      taskId: task.id,
      lane: task.lane,
      title: task.title,
      status: task.status,
      provider: task.provider,
      sharedPaths,
      changedPaths: task.changedPaths.length
    });
  }

  return overlaps
    .sort((left, right) => right.sharedPaths.length - left.sharedPaths.length || left.title.localeCompare(right.title))
    .slice(0, MAX_OVERLAPS);
}

function sameTask(left: TaskOverlapInput, right: TaskOverlapInput): boolean {
  return left.id === right.id || left.lane === right.lane;
}

function isInactive(status: string): boolean {
  return INACTIVE_STATUSES.has(status.trim().toLowerCase());
}

function normalizePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/^\.\//, "");
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    if (!value || seen.has(value)) {
      continue;
    }
    seen.add(value);
    result.push(value);
  }
  return result;
}
