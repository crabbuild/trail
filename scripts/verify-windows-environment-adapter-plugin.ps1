$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw "Windows is required for the AppContainer adapter-plugin verifier"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    cargo build -p trail
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    cargo build -p trail-environment-adapter-sdk --example generated-copy-adapter --example mounted-initializer-adapter --example mounted-fixture-tool --example cache-adapter --example cache-fixture-tool --example adversarial-adapter --example fixture-sign-adapter
    if ($LASTEXITCODE -ne 0) { throw "adapter example build failed" }
    $trail = Join-Path $repoRoot "target/debug/trail.exe"
    $exampleDir = Join-Path $repoRoot "target/debug/examples"

    function Write-Package(
        [string] $Directory,
        [string] $Identity,
        [string] $Selector,
        [string] $ExecutableSource,
        [int] $TimeoutMs,
        [int] $ResponseBytes
    ) {
        New-Item -ItemType Directory -Path $Directory -Force | Out-Null
        $executable = Join-Path $Directory "adapter-plugin.exe"
        Copy-Item -LiteralPath $ExecutableSource -Destination $executable
        $digest = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
        $manifest = @"
schema = "trail.environment-adapter-package/v1"

[adapter]
canonical_identity = "$Identity"
implementation_version = "1.0.0"
selectors = ["$Identity", "$Selector"]
kind = "generated"
layer_adapter_name = "$Selector"
discovery_markers = ["copy.adapter"]
stability = "experimental"
description = "Windows fixture for $Identity"

[executable]
path = "adapter-plugin.exe"
sha256 = "$digest"

[permissions]
read_patterns = ["copy.adapter", "input.txt"]
max_input_files = 8
max_input_bytes = 1048576
timeout_ms = $TimeoutMs
max_response_bytes = $ResponseBytes
"@
        Set-Content -LiteralPath (Join-Path $Directory "trail-adapter.toml") -Value $manifest -Encoding utf8NoBOM
    }

    function Assert-TrailFails([scriptblock] $Action, [string] $Message) {
        & $Action *> $null
        if ($LASTEXITCODE -eq 0) { throw $Message }
    }

    function Write-MountedPackage([string] $Directory) {
        New-Item -ItemType Directory -Path $Directory -Force | Out-Null
        $executable = Join-Path $Directory "adapter-plugin.exe"
        Copy-Item -LiteralPath (Join-Path $exampleDir "mounted-initializer-adapter.exe") -Destination $executable
        $digest = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
        $manifest = @"
schema = "trail.environment-adapter-package/v1"

[adapter]
canonical_identity = "example/mounted@1"
implementation_version = "1.0.0"
selectors = ["example/mounted@1", "example-mounted"]
kind = "generated"
layer_adapter_name = "example-mounted"
discovery_markers = ["mounted.adapter"]
protocols = ["trail.environment-adapter/v2"]
stability = "experimental"
description = "Windows mounted initializer protocol-v2 fixture"

[executable]
path = "adapter-plugin.exe"
sha256 = "$digest"

[permissions]
read_patterns = ["mounted.adapter"]
max_input_files = 8
max_input_bytes = 1048576
timeout_ms = 5000
max_response_bytes = 1048576
"@
        Set-Content -LiteralPath (Join-Path $Directory "trail-adapter.toml") -Value $manifest -Encoding utf8NoBOM
    }

    function Write-CachePackage([string] $Directory) {
        New-Item -ItemType Directory -Path $Directory -Force | Out-Null
        $executable = Join-Path $Directory "adapter-plugin.exe"
        Copy-Item -LiteralPath (Join-Path $exampleDir "cache-adapter.exe") -Destination $executable
        $digest = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
        $manifest = @"
schema = "trail.environment-adapter-package/v1"

[adapter]
canonical_identity = "example/cache@1"
implementation_version = "1.0.0"
selectors = ["example/cache@1", "example-cache"]
kind = "generated"
layer_adapter_name = "example-cache"
discovery_markers = ["cache.adapter"]
protocols = ["trail.environment-adapter/v2"]
stability = "experimental"
description = "Windows host-owned cache protocol-v2 fixture"

[executable]
path = "adapter-plugin.exe"
sha256 = "$digest"

[permissions]
read_patterns = ["cache.adapter"]
max_input_files = 8
max_input_bytes = 1048576
timeout_ms = 5000
max_response_bytes = 1048576
"@
        Set-Content -LiteralPath (Join-Path $Directory "trail-adapter.toml") -Value $manifest -Encoding utf8NoBOM
    }

    $workspace = Join-Path $env:RUNNER_TEMP ("trail-plugin-" + [guid]::NewGuid())
    $packages = Join-Path $env:RUNNER_TEMP ("trail-plugin-packages-" + [guid]::NewGuid())
    $toolBin = Join-Path $env:RUNNER_TEMP ("trail-plugin-tools-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $workspace, $packages, $toolBin -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $exampleDir "mounted-fixture-tool.exe") -Destination (Join-Path $toolBin "mounted-fixture-tool.exe")
    Copy-Item -LiteralPath (Join-Path $exampleDir "cache-fixture-tool.exe") -Destination (Join-Path $toolBin "cache-fixture-tool.exe")
    $env:PATH = "$toolBin;$($env:PATH)"
    Set-Content -LiteralPath (Join-Path $workspace "copy.adapter") -Value "plugin marker" -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $workspace "mounted.adapter") -Value "success" -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $workspace "cache.adapter") -Value "lane-a" -Encoding utf8NoBOM
    Set-Content -LiteralPath (Join-Path $workspace "input.txt") -Value "declared input" -Encoding utf8NoBOM

    $copyPackage = Join-Path $packages "copy"
    $mountedPackage = Join-Path $packages "mounted"
    $cachePackage = Join-Path $packages "cache"
    Write-Package $copyPackage "example/copy@1" "example-copy" (Join-Path $exampleDir "generated-copy-adapter.exe") 5000 1048576
    Write-MountedPackage $mountedPackage
    Write-CachePackage $cachePackage
    & $trail --workspace $workspace init --working-tree | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "trail init failed" }
    $inspection = (& $trail --workspace $workspace --json env plugin inspect $copyPackage) | ConvertFrom-Json
    $signaturePath = Join-Path $copyPackage "trail-adapter.sig"
    $keyPath = Join-Path $copyPackage "publisher-key.toml"
    & (Join-Path $exampleDir "fixture-sign-adapter.exe") `
        "example-publisher" `
        "0707070707070707070707070707070707070707070707070707070707070707" `
        $inspection.payload_digest `
        $signaturePath `
        $keyPath
    if ($LASTEXITCODE -ne 0) { throw "adapter fixture signing failed" }
    Assert-TrailFails { & $trail --workspace $workspace env plugin install $copyPackage } "signed Windows adapter installed before publisher trust"
    $trusted = (& $trail --workspace $workspace --json env plugin trust add $keyPath) | ConvertFrom-Json
    foreach ($lane in @("plugin-a", "plugin-b", "plugin-mounted-a", "plugin-mounted-b", "plugin-mounted-kill")) {
        & $trail --workspace $workspace lane spawn $lane --from main --workdir-mode dokan-cow | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "lane spawn failed for $lane" }
    }

    $installed = (& $trail --workspace $workspace --json env plugin install $copyPackage) | ConvertFrom-Json
    if ($installed.trust -ne "publisher_signed" -or $installed.certification_tier -ne "publisher-authenticated-experimental") {
        throw "signed Windows adapter did not report publisher authentication"
    }
    $catalog = (& $trail --workspace $workspace --json env adapters) | ConvertFrom-Json
    $catalogEntry = $catalog.adapters | Where-Object { $_.canonical_identity -eq "example/copy@1" }
    if ($null -eq $catalogEntry -or $catalogEntry.source -ne "plugin") {
        throw "installed Windows plugin was absent from the adapter catalog"
    }
    $discovery = (& $trail --workspace $workspace --json env discover plugin-a) | ConvertFrom-Json
    if ($discovery.components.component_id -notcontains "plugin.copy") {
        throw "Windows plugin discovery did not propose plugin.copy"
    }
    $plan = (& $trail --workspace $workspace --json env plan plugin-a --adapter example/copy@1) | ConvertFrom-Json
    if ($plan.capabilities.sandbox -ne "windows-appcontainer-job" -or $plan.capabilities.network -ne "deny") {
        throw "Windows plugin plan did not report denied-by-default AppContainer capabilities"
    }
    $first = (& $trail --workspace $workspace --json env sync plugin-a --adapter example/copy@1) | ConvertFrom-Json
    $second = (& $trail --workspace $workspace --json env sync plugin-b --adapter example/copy@1) | ConvertFrom-Json
    $firstLayer = $first.layers[0]
    $secondLayer = $second.layers[0]
    if ($firstLayer.layer_id -ne $secondLayer.layer_id) { throw "Windows plugins did not reuse one layer" }
    if (-not (Test-Path -LiteralPath (Join-Path $firstLayer.storage_path "out.tar") -PathType Leaf)) {
        throw "Windows plugin did not publish its generated archive"
    }
    & $trail --workspace $workspace lane exec plugin-a -- cmd.exe /d /c "echo lane-a>.trail-generated\plugin-copy\private.txt"
    if ($LASTEXITCODE -ne 0) { throw "could not mutate the first plugin lane" }
    & $trail --workspace $workspace lane exec plugin-b -- cmd.exe /d /c "if exist .trail-generated\plugin-copy\private.txt (exit /b 9) else (exit /b 0)"
    if ($LASTEXITCODE -ne 0) { throw "Windows plugin private output leaked between lanes" }
    & $trail --workspace $workspace lane exec plugin-a -- cmd.exe /d /c "echo changed-input>input.txt"
    if ($LASTEXITCODE -ne 0) { throw "could not change the Windows plugin identity input" }
    & $trail --workspace $workspace lane checkpoint plugin-a -m "change plugin input" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not checkpoint the Windows plugin identity input" }
    $readiness = (& $trail --workspace $workspace --json lane readiness plugin-a) | ConvertFrom-Json
    if ($readiness.blockers.code -notcontains "dependency_environment_stale") {
        throw "Windows plugin input change did not block readiness as stale"
    }
    $status = (& $trail --workspace $workspace --json env status plugin-a) | ConvertFrom-Json
    if ($status.status -notcontains "stale") { throw "Windows plugin state did not persist stale" }

    & $trail --workspace $workspace env plugin install $cachePackage | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not install Windows cache fixture" }
    & $trail --workspace $workspace lane exec plugin-b -- cmd.exe /d /c "echo lane-b>cache.adapter" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not change the second Windows cache fixture input" }
    & $trail --workspace $workspace lane checkpoint plugin-b -m "give cache fixture a distinct component key" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not checkpoint the second Windows cache fixture input" }
    $cachePlan = (& $trail --workspace $workspace --json env plan plugin-a --adapter example/cache@1) | ConvertFrom-Json
    if ($cachePlan.caches.Count -ne 1 -or $cachePlan.caches[0].access -ne "host_exclusive" -or $cachePlan.caches[0].protocol -ne "content_store") {
        throw "Windows cache plugin plan lost its conservative cache contract"
    }
    $cacheA = (& $trail --workspace $workspace --json env sync plugin-a --adapter example/cache@1) | ConvertFrom-Json
    $cacheB = (& $trail --workspace $workspace --json env sync plugin-b --adapter example/cache@1) | ConvertFrom-Json
    $cacheANamespace = $cacheA.generation.components[0].caches[0].namespace_id
    $cacheBNamespace = $cacheB.generation.components[0].caches[0].namespace_id
    if ($cacheANamespace -ne $cacheBNamespace) { throw "Windows cache plugin lanes did not reuse one namespace" }
    $cacheAObservation = @(& $trail --workspace $workspace lane exec plugin-a -- cmd.exe /d /c "type .trail-generated\plugin-cache\cache-observation.txt")[0].Trim()
    $cacheBObservation = @(& $trail --workspace $workspace lane exec plugin-b -- cmd.exe /d /c "type .trail-generated\plugin-cache\cache-observation.txt")[0].Trim()
    if ($cacheAObservation -ne "$cacheANamespace|1" -or $cacheBObservation -ne "$cacheBNamespace|2") {
        throw "Windows cache plugin did not serialize and reuse its host namespace"
    }
    $counter = (Get-Content -LiteralPath (Join-Path $workspace ".trail/cache/namespaces/$cacheANamespace/counter") -Raw).Trim()
    if ($counter -ne "2") { throw "Windows cache plugin namespace did not retain both executions" }
    & $trail --workspace $workspace lane exec plugin-a -- cmd.exe /d /c "echo escape>cache.adapter" | Out-Null
    & $trail --workspace $workspace lane checkpoint plugin-a -m "attempt plugin cache namespace escape" | Out-Null
    Assert-TrailFails { & $trail --workspace $workspace env sync plugin-a --adapter example/cache@1 } "Windows plugin cache write escaped its namespace"
    if (Test-Path -LiteralPath (Join-Path $workspace ".trail/cache/namespaces/plugin-cache-escape")) {
        throw "Windows plugin cache escape created a sibling namespace entry"
    }
    $cacheAObservation = @(& $trail --workspace $workspace lane exec plugin-a -- cmd.exe /d /c "type .trail-generated\plugin-cache\cache-observation.txt")[0].Trim()
    if ($cacheAObservation -ne "$cacheANamespace|1") { throw "failed Windows cache escape replaced its predecessor" }

    & $trail --workspace $workspace env plugin install $mountedPackage | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not install mounted protocol-v2 fixture" }
    $mountedCatalog = (& $trail --workspace $workspace --json env adapters) | ConvertFrom-Json
    $mountedEntry = $mountedCatalog.adapters | Where-Object { $_.canonical_identity -eq "example/mounted@1" }
    if ($null -eq $mountedEntry -or $mountedEntry.protocols -notcontains "trail.environment-adapter/v2") {
        throw "Windows catalog did not expose the mounted adapter protocol"
    }
    $mountedPlan = (& $trail --workspace $workspace --json env plan plugin-mounted-a --adapter example/mounted@1) | ConvertFrom-Json
    if ($mountedPlan.commands.Count -ne 1 -or $mountedPlan.commands[0].phase -ne "mounted_initialization") {
        throw "Windows mounted plugin did not report exactly one mounted action"
    }
    $mountedA = (& $trail --workspace $workspace --json env sync plugin-mounted-a --adapter example/mounted@1) | ConvertFrom-Json
    $mountedB = (& $trail --workspace $workspace --json env sync plugin-mounted-b --adapter example/mounted@1) | ConvertFrom-Json
    if ($mountedA.layers.Count -ne 0 -or $mountedB.layers.Count -ne 0) {
        throw "Windows mounted plugin unexpectedly published a shared layer"
    }
    $mountedAPwd = @(& $trail --workspace $workspace lane exec plugin-mounted-a -- cmd.exe /d /c cd)[0].Trim()
    $mountedBPwd = @(& $trail --workspace $workspace lane exec plugin-mounted-b -- cmd.exe /d /c cd)[0].Trim()
    $mountedARecorded = @(& $trail --workspace $workspace lane exec plugin-mounted-a -- cmd.exe /d /c "type .trail-generated\plugin-mounted\initialized.txt")[0].Trim()
    $mountedBRecorded = @(& $trail --workspace $workspace lane exec plugin-mounted-b -- cmd.exe /d /c "type .trail-generated\plugin-mounted\initialized.txt")[0].Trim()
    $mountedARecorded = $mountedARecorded -replace '\|success$', ''
    $mountedBRecorded = $mountedBRecorded -replace '\|success$', ''
    if ($mountedARecorded -ne $mountedAPwd -or $mountedBRecorded -ne $mountedBPwd -or $mountedARecorded -eq $mountedBRecorded) {
        throw "Windows mounted plugin did not initialize at two distinct final lane paths"
    }
    & $trail --workspace $workspace lane exec plugin-mounted-a -- cmd.exe /d /c "echo lane-a-private>.trail-generated\plugin-mounted\initialized.txt" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not mutate first Windows mounted output" }
    & $trail --workspace $workspace env sync plugin-mounted-a --adapter example/mounted@1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "compatible Windows mounted resync failed" }
    $preserved = @(& $trail --workspace $workspace lane exec plugin-mounted-a -- cmd.exe /d /c "type .trail-generated\plugin-mounted\initialized.txt")[0].Trim()
    if ($preserved -ne "lane-a-private") { throw "compatible Windows mounted resync replaced private state" }
    & $trail --workspace $workspace lane exec plugin-mounted-a -- cmd.exe /d /c "echo fail>mounted.adapter" | Out-Null
    & $trail --workspace $workspace lane checkpoint plugin-mounted-a -m "fail mounted plugin action" | Out-Null
    Assert-TrailFails { & $trail --workspace $workspace env sync plugin-mounted-a --adapter example/mounted@1 } "failed Windows mounted action unexpectedly activated"
    $preserved = @(& $trail --workspace $workspace lane exec plugin-mounted-a -- cmd.exe /d /c "type .trail-generated\plugin-mounted\initialized.txt")[0].Trim()
    if ($preserved -ne "lane-a-private") { throw "failed Windows mounted action replaced its predecessor" }
    & $trail --workspace $workspace lane exec plugin-mounted-b -- cmd.exe /d /c "echo source_write>mounted.adapter" | Out-Null
    & $trail --workspace $workspace lane checkpoint plugin-mounted-b -m "attempt mounted source write" | Out-Null
    Assert-TrailFails { & $trail --workspace $workspace env sync plugin-mounted-b --adapter example/mounted@1 } "Windows mounted source write escaped its output contract"
    & $trail --workspace $workspace lane exec plugin-mounted-b -- cmd.exe /d /c "if exist source-leak.txt (exit /b 9) else (exit /b 0)" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Windows mounted source write leaked into the lane" }
    & $trail --workspace $workspace lane exec plugin-mounted-b -- cmd.exe /d /c "echo source_read>mounted.adapter" | Out-Null
    & $trail --workspace $workspace lane checkpoint plugin-mounted-b -m "attempt undeclared mounted source read" | Out-Null
    Assert-TrailFails { & $trail --workspace $workspace env sync plugin-mounted-b --adapter example/mounted@1 } "Windows mounted undeclared source read unexpectedly succeeded"
    & $trail --workspace $workspace lane exec plugin-mounted-b -- cmd.exe /d /c "if exist .trail-generated\plugin-mounted\leaked.txt (exit /b 9) else (exit /b 0)" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Windows mounted undeclared source read leaked content" }
    & $trail --workspace $workspace env sync plugin-mounted-kill --adapter example/mounted@1 | Out-Null
    & $trail --workspace $workspace lane exec plugin-mounted-kill -- cmd.exe /d /c "echo kill-predecessor>.trail-generated\plugin-mounted\initialized.txt & echo hang>mounted.adapter" | Out-Null
    & $trail --workspace $workspace lane checkpoint plugin-mounted-kill -m "kill active mounted plugin action" | Out-Null
    $killOut = Join-Path $packages "mounted-kill.stdout"
    $killErr = Join-Path $packages "mounted-kill.stderr"
    $syncProcess = Start-Process -FilePath $trail -ArgumentList @(
        "--workspace", $workspace, "env", "sync", "plugin-mounted-kill",
        "--adapter", "example/mounted@1"
    ) -RedirectStandardOutput $killOut -RedirectStandardError $killErr -PassThru
    $readyFile = $null
    for ($attempt = 0; $attempt -lt 200; $attempt++) {
        $readyFile = Get-ChildItem -LiteralPath (Join-Path $workspace ".trail/cache/staging") -Filter "running" -File -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($null -ne $readyFile) { break }
        Start-Sleep -Milliseconds 50
    }
    if ($null -eq $readyFile) {
        Stop-Process -Id $syncProcess.Id -Force -ErrorAction SilentlyContinue
        throw "Windows mounted plugin did not reach its active-command kill point"
    }
    $mountedChildPid = [int]((Get-Content -LiteralPath $readyFile.FullName -Raw).Trim())
    Stop-Process -Id $syncProcess.Id -Force
    $syncProcess.WaitForExit()
    for ($attempt = 0; $attempt -lt 200; $attempt++) {
        if ($null -eq (Get-Process -Id $mountedChildPid -ErrorAction SilentlyContinue)) { break }
        Start-Sleep -Milliseconds 50
    }
    if ($null -ne (Get-Process -Id $mountedChildPid -ErrorAction SilentlyContinue)) {
        Stop-Process -Id $mountedChildPid -Force -ErrorAction SilentlyContinue
        throw "Windows mounted plugin action survived Trail process death"
    }
    & $trail --workspace $workspace env status plugin-mounted-kill | Out-Null
    $preserved = @(& $trail --workspace $workspace lane exec plugin-mounted-kill -- cmd.exe /d /c "type .trail-generated\plugin-mounted\initialized.txt")[0].Trim()
    if ($preserved -ne "kill-predecessor") { throw "Windows active-command kill replaced its predecessor" }
    $abandoned = Get-ChildItem -LiteralPath (Join-Path $workspace ".trail/cache/staging") -Directory -Filter "mounted-environment-*" -ErrorAction SilentlyContinue
    if ($null -ne $abandoned) { throw "Windows recovery left an abandoned mounted plugin candidate" }

    foreach ($behavior in @("hang", "crash", "oversized", "malformed", "child", "memory")) {
        $timeout = if ($behavior -eq "hang") { 100 } else { 1000 }
        $package = Join-Path $packages $behavior
        Write-Package $package "example/$behavior@1" "example-$behavior" (Join-Path $exampleDir "adversarial-adapter.exe") $timeout 1048576
        & $trail --workspace $workspace env plugin install $package | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "could not install $behavior fixture" }
        Assert-TrailFails { & $trail --workspace $workspace env plan plugin-a --adapter "example/$behavior@1" } "adversarial Windows $behavior adapter unexpectedly succeeded"
    }

    $removed = (& $trail --workspace $workspace --json env plugin remove example/copy@1) | ConvertFrom-Json
    if ($removed.removed_distribution_digest -ne $installed.distribution_digest) {
        throw "Windows plugin removal lost its active distribution provenance"
    }
    $reinstalled = (& $trail --workspace $workspace --json env plugin install $copyPackage) | ConvertFrom-Json
    Add-Content -LiteralPath (Join-Path $reinstalled.package_path "adapter-plugin.exe") -Value "tamper" -Encoding utf8NoBOM
    Assert-TrailFails { & $trail --workspace $workspace env adapters } "tampered Windows adapter executable remained trusted"
    & $trail --workspace $workspace env plugin install $copyPackage | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not repair the tampered Windows plugin" }
    & $trail --workspace $workspace env plugin trust remove $trusted.key_id | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not revoke the Windows publisher key" }
    Assert-TrailFails { & $trail --workspace $workspace env adapters } "revoked Windows publisher key left signed adapter active"
    & $trail --workspace $workspace env plugin remove example/copy@1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not tombstone the tampered Windows plugin" }
    & $trail --workspace $workspace env adapters | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "catalog did not recover after plugin tombstone" }

    Write-Output "windows-environment-adapter-plugin distribution=$($installed.distribution_digest) shared-layer=$($firstLayer.layer_id) external-cache=${cacheANamespace}:shared-host-exclusive-and-sandbox-contained mounted-v2=isolated-and-atomic active-command-kill=terminated-and-recovered declared-read=allowed undeclared-read-write=denied private-copy-up=isolated stale-refresh=verified publisher-signature=verified revocation=fail-closed timeout=denied memory=denied crash=denied oversized=denied malformed=denied child-process=denied tamper=denied"
}
finally {
    Pop-Location
}
