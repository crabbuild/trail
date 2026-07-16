$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw "Windows is required for the AppContainer command-recipe verifier"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    cargo build -p trail
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    $trail = Join-Path $repoRoot "target/debug/trail.exe"

    function Convert-ToTomlString([string] $Value) {
        return '"' + $Value.Replace('\', '\\').Replace('"', '\"') + '"'
    }

    function Write-Recipe([string] $Root, [string[]] $Command, [string] $Policy = "immutable_seed_private") {
        New-Item -ItemType Directory -Path $Root -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $Root "input.txt") -Value "declared input" -Encoding utf8NoBOM
        $commandToml = ($Command | ForEach-Object { Convert-ToTomlString $_ }) -join ", "
        $specification = @"
schema = "trail.environment/v1"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "generated.copy"
adapter = "trail/command@1"
root = "."
kind = "generated"

[[component.input]]
path = "input.txt"
role = "identity"
format = "bytes"

[component.build]
command = [$commandToml]
cwd = "."
network = "deny"
scripts = "deny"

[[component.output]]
name = "generated"
source = "generated"
target = ".trail-generated/copy"
policy = "$Policy"
portability = "host"
"@
        Set-Content -LiteralPath (Join-Path $Root "trail.environment.toml") -Value $specification -Encoding utf8NoBOM
    }

    function New-RecipeLane([string] $Root, [string] $Name = "recipe-a") {
        & $trail --workspace $Root init --working-tree | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "trail init failed for $Root" }
        & $trail --workspace $Root lane spawn $Name --from main --workdir-mode dokan-cow | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "lane spawn failed for $Root" }
    }

    function Assert-TrailFails([scriptblock] $Action, [string] $Message) {
        & $Action *> $null
        if ($LASTEXITCODE -eq 0) { throw $Message }
    }

    $successRoot = Join-Path $env:RUNNER_TEMP ("trail-recipe-success-" + [guid]::NewGuid())
    Write-Recipe $successRoot @("tar.exe", "-cf", "generated/out.tar", "input.txt")
    New-RecipeLane $successRoot
    & $trail --workspace $successRoot lane spawn recipe-b --from main --workdir-mode dokan-cow | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "second lane spawn failed" }
    $plan = (& $trail --workspace $successRoot --json env plan recipe-a --adapter command) | ConvertFrom-Json
    if ($plan.capabilities.sandbox -ne "windows-appcontainer-job") {
        throw "plan did not report the Windows AppContainer sandbox"
    }
    $first = (& $trail --workspace $successRoot --json env sync recipe-a --adapter command) | ConvertFrom-Json
    $second = (& $trail --workspace $successRoot --json env sync recipe-b --adapter command) | ConvertFrom-Json
    $firstLayer = $first.layers[0]
    $secondLayer = $second.layers[0]
    if ($firstLayer.layer_id -ne $secondLayer.layer_id) { throw "identical Windows recipes did not reuse one layer" }
    if (-not (Test-Path -LiteralPath (Join-Path $firstLayer.storage_path "out.tar") -PathType Leaf)) {
        throw "sandboxed Windows recipe did not publish its declared archive"
    }

    $privateRoot = Join-Path $env:RUNNER_TEMP ("trail-recipe-private-" + [guid]::NewGuid())
    Write-Recipe $privateRoot @("tar.exe", "-cf", "generated/out.tar", "input.txt") "writable_private"
    New-RecipeLane $privateRoot
    $private = (& $trail --workspace $privateRoot --json env sync recipe-a --adapter command) | ConvertFrom-Json
    if ($private.layers.Count -ne 0) { throw "writable-private Windows recipe manufactured a shared layer" }
    $privateOutput = $private.generation.components[0].outputs[0]
    if ($privateOutput.policy -ne "writable_private" -or $null -ne $privateOutput.layer_id) {
        throw "writable-private Windows recipe reported incorrect storage policy"
    }
    & $trail --workspace $privateRoot lane exec recipe-a -- cmd.exe /d /c "echo private-mutation>.trail-generated\copy\private.txt"
    if ($LASTEXITCODE -ne 0) { throw "could not mutate writable-private Windows output" }
    & $trail --workspace $privateRoot env sync recipe-a --adapter command | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not resynchronize writable-private Windows output" }
    & $trail --workspace $privateRoot lane exec recipe-a -- cmd.exe /d /c "findstr private-mutation .trail-generated\copy\private.txt>nul"
    if ($LASTEXITCODE -ne 0) { throw "writable-private Windows state was not preserved" }

    $multiRoot = Join-Path $env:RUNNER_TEMP ("trail-recipe-multi-" + [guid]::NewGuid())
    Write-Recipe $multiRoot @("tar.exe", "-cf", "generated/out.tar", "input.txt")
    Add-Content -LiteralPath (Join-Path $multiRoot "trail.environment.toml") -Encoding utf8NoBOM -Value @"

[[component.output]]
name = "beta"
source = "generated-b"
target = ".trail-generated/beta"
policy = "immutable_seed_private"
portability = "host"
"@
    New-RecipeLane $multiRoot
    & $trail --workspace $multiRoot lane spawn recipe-b --from main --workdir-mode dokan-cow | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "multi-output second lane spawn failed" }
    $multiFirst = (& $trail --workspace $multiRoot --json env sync recipe-a --adapter command) | ConvertFrom-Json
    $multiSecond = (& $trail --workspace $multiRoot --json env sync recipe-b --adapter command) | ConvertFrom-Json
    $multiFirstLayer = $multiFirst.layers[0]
    $multiSecondLayer = $multiSecond.layers[0]
    if ($multiFirstLayer.layer_id -ne $multiSecondLayer.layer_id) {
        throw "identical Windows multi-output recipes did not reuse one layer"
    }
    if ($multiFirst.generation.components[0].outputs.Count -ne 2) {
        throw "Windows recipe did not report both declared outputs"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $multiFirstLayer.storage_path "outputs/0000/out.tar") -PathType Leaf)) {
        throw "Windows recipe did not package its first composite output"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $multiFirstLayer.storage_path "outputs/0001") -PathType Container)) {
        throw "Windows recipe did not package its second composite output"
    }
    & $trail --workspace $multiRoot lane exec recipe-a -- cmd.exe /d /c "echo lane-a>.trail-generated\beta\lane.txt"
    if ($LASTEXITCODE -ne 0) { throw "could not mutate the first lane's private multi-output upper" }
    & $trail --workspace $multiRoot lane exec recipe-b -- cmd.exe /d /c "if exist .trail-generated\beta\lane.txt (exit /b 9) else (exit /b 0)"
    if ($LASTEXITCODE -ne 0) { throw "a Windows multi-output mutation leaked between lanes" }

    $outside = Join-Path $env:RUNNER_TEMP ("trail-outside-" + [guid]::NewGuid() + ".txt")
    Set-Content -LiteralPath $outside -Value "host canary" -Encoding utf8NoBOM
    & "$env:SystemRoot/System32/icacls.exe" $outside /inheritance:r /grant:r "${env:USERNAME}:(R,W)" "SYSTEM:(F)" /Q | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not harden the outside-read canary ACL" }
    $readRoot = Join-Path $env:RUNNER_TEMP ("trail-recipe-read-" + [guid]::NewGuid())
    Write-Recipe $readRoot @("tar.exe", "-cf", "generated/out.tar", $outside)
    New-RecipeLane $readRoot
    Assert-TrailFails { & $trail --workspace $readRoot env sync recipe-a --adapter command } "restricted Windows recipe unexpectedly read an undeclared host file"

    $writeRoot = Join-Path $env:RUNNER_TEMP ("trail-recipe-write-" + [guid]::NewGuid())
    Write-Recipe $writeRoot @("tar.exe", "-cf", "escape.tar", "input.txt")
    New-RecipeLane $writeRoot
    Assert-TrailFails { & $trail --workspace $writeRoot env sync recipe-a --adapter command } "restricted Windows recipe unexpectedly wrote outside its declared output"

    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = ([System.Net.IPEndPoint] $listener.LocalEndpoint).Port
    $listener.Stop()
    $server = Start-Process -FilePath "python.exe" -ArgumentList @("-m", "http.server", "$port", "--bind", "127.0.0.1") -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Milliseconds 500
        $networkRoot = Join-Path $env:RUNNER_TEMP ("trail-recipe-network-" + [guid]::NewGuid())
        Write-Recipe $networkRoot @("curl.exe", "--fail", "--max-time", "2", "http://127.0.0.1:$port/", "-o", "generated/network.txt")
        New-RecipeLane $networkRoot
        Assert-TrailFails { & $trail --workspace $networkRoot env sync recipe-a --adapter command } "restricted Windows recipe unexpectedly used a network socket"
    }
    finally {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }

    $childRoot = Join-Path $env:RUNNER_TEMP ("trail-recipe-child-" + [guid]::NewGuid())
    Write-Recipe $childRoot @("forfiles.exe", "/P", ".", "/M", "input.txt", "/C", "cmd /c echo child>generated\child.txt")
    New-RecipeLane $childRoot
    & $trail --workspace $childRoot --json env sync recipe-a --adapter command *> $null
    if ($LASTEXITCODE -eq 0) {
        $childLayer = (& $trail --workspace $childRoot --json env sync recipe-a --adapter command) | ConvertFrom-Json
        if (Test-Path -LiteralPath (Join-Path $childLayer.layers[0].storage_path "child.txt") -PathType Leaf) {
            throw "restricted Windows recipe unexpectedly launched a child process"
        }
    }

    $shellRoot = Join-Path $env:RUNNER_TEMP ("trail-recipe-shell-" + [guid]::NewGuid())
    Write-Recipe $shellRoot @("cmd.exe", "/c", "exit", "0")
    New-RecipeLane $shellRoot
    Assert-TrailFails { & $trail --workspace $shellRoot env plan recipe-a --adapter command } "restricted Windows recipe unexpectedly accepted cmd.exe"

    Write-Output "windows-command-recipe shared-layer=$($firstLayer.layer_id) multi-output-layer=$($multiFirstLayer.layer_id) writable-private=verified private-copy-up=isolated host-read=denied undeclared-write=denied network=denied child-process=denied shell=denied"
}
finally {
    Pop-Location
}
