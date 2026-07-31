$ErrorActionPreference = "Stop"

$target = $env:TARGET
if ([string]::IsNullOrWhiteSpace($target)) {
  throw "TARGET is required"
}

if ($env:GITHUB_REF_TYPE -eq "tag") {
  $version = $env:GITHUB_REF_NAME
} else {
  $version = "manual-$env:GITHUB_RUN_ID"
}

$archive = "rime-llm-$version-windows-x86_64.zip"
$package = Join-Path $env:RUNNER_TEMP "rime-llm-package"
Remove-Item -Recurse -Force package-output, $package -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force package-output, $package | Out-Null
Copy-Item "target\$target\release\rime-llm.exe" (Join-Path $package "rime-llm.exe")
Compress-Archive -Path (Join-Path $package "rime-llm.exe") -DestinationPath (Join-Path "package-output" $archive)
$hash = (Get-FileHash -Algorithm SHA256 (Join-Path "package-output" $archive)).Hash.ToLowerInvariant()
"$hash  $archive" | Set-Content -NoNewline (Join-Path "package-output" "$archive.sha256")
