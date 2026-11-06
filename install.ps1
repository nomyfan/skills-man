# Install the latest skills-man release (the `skill` binary) on Windows.
#
#   irm https://raw.githubusercontent.com/nomyfan/skills-man/main/install.ps1 | iex
#
# Set $env:SKILLS_MAN_INSTALL_DIR to change where skill.exe goes
# (default: ~\.local\bin). The directory is added to the user PATH.

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest is much slower with the progress bar on Windows PowerShell 5.1.
$ProgressPreference = 'SilentlyContinue'
# Windows PowerShell 5.1 may not enable TLS 1.2 by default, which GitHub requires.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = 'nomyfan/skills-man'
# Only an x86_64 build is published; ARM64 Windows runs it through emulation.
$target = 'x86_64-pc-windows-msvc'
$installDir = if ($env:SKILLS_MAN_INSTALL_DIR) { $env:SKILLS_MAN_INSTALL_DIR } else { Join-Path $HOME '.local\bin' }
$url = "https://github.com/$repo/releases/latest/download/skill-$target.zip"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
    Write-Host "Downloading $url"
    $zip = Join-Path $tmp 'skill.zip'
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
    Expand-Archive -Path $zip -DestinationPath $tmp -Force

    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $exe = Join-Path $installDir 'skill.exe'
    # The archive holds a single file named after the target, e.g. skill-x86_64-pc-windows-msvc.exe.
    Move-Item -Path (Join-Path $tmp "skill-$target.exe") -Destination $exe -Force

    Write-Host "Installed $(& $exe --version) to $exe"
}
finally {
    Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$entries = if ($userPath) { $userPath -split ';' } else { @() }
if (($entries | ForEach-Object { $_.TrimEnd('\') }) -notcontains $installDir.TrimEnd('\')) {
    $newPath = (@($entries | Where-Object { $_ }) + $installDir) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    $env:Path = "$env:Path;$installDir"
    Write-Host "Added $installDir to your user PATH. Restart your terminal to pick it up."
}
