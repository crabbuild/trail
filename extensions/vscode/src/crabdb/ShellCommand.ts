export function shellCommandForPlatform(commandLine: string, platform: NodeJS.Platform = process.platform): string[] {
  const command = commandLine.trim();
  if (!command) {
    throw new Error("Command cannot be empty.");
  }
  if (platform === "win32") {
    return ["cmd.exe", "/d", "/s", "/c", command];
  }
  return ["sh", "-lc", command];
}
