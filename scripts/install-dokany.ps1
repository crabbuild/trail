$ErrorActionPreference = "Stop"

$version = "2.0.6.1000"
$expectedSha256 = "1DE58167E28D0C4BE6AF17ABFE5CE9D8DC0BFF032F900B225E23B79147B0FFF2"
$installer = Join-Path $env:RUNNER_TEMP "Dokan_x64-$version.msi"
$uri = "https://github.com/dokan-dev/dokany/releases/download/v$version/Dokan_x64.msi"

Invoke-WebRequest -Uri $uri -OutFile $installer
$actualSha256 = (Get-FileHash -Path $installer -Algorithm SHA256).Hash
if ($actualSha256 -ne $expectedSha256) {
    throw "Dokany installer checksum mismatch: expected $expectedSha256, got $actualSha256"
}

$arguments = @("/i", "`"$installer`"", "/qn", "/norestart")
$process = Start-Process -FilePath "msiexec.exe" -ArgumentList $arguments -Wait -PassThru
if ($process.ExitCode -notin @(0, 3010)) {
    throw "Dokany installer failed with exit code $($process.ExitCode)"
}
