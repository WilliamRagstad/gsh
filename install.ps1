<#
.SYNOPSIS
!    Install the latest Client binary for Windows
!    1. Detects system architecture before downloading the correct version.
!    2. Downloads the latest gsh binary from GitHub.
!    3. Adds it to the PATH variable.
!
! Copyright (c) 2023 William Ragstad
! Licensed under the MIT License.
#>

if (-not ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)) {
	Write-Host "This script requires administrative privileges. Please run as administrator." -ForegroundColor Red
	exit 1
}

if ([System.Environment]::Is64BitOperatingSystem) { $arch = "x64" }
else { $arch = "x86" }

$release_api = "https://api.github.com/repos/WilliamRagstad/gsh/releases/latest"
$release = Invoke-RestMethod -Uri $release_api -Headers @{ "User-Agent" = "gsh-installer" }
$version = $release.tag_name.TrimStart("v")   # e.g. 4.14.1
$asset = $release.assets | Where-Object {
	$_.name -like "*$version*" -and
	$_.name -like "*$arch*" -and
	$_.name -like "*win*" -and
	$_.name -like "*.exe"
} | Select-Object -First 1

if (-not $asset) {
	throw "Could not find $arch windows binary in latest release."
}

$gsh_dir = "$env:LOCALAPPDATA\gsh"
$gsh_bin = "$gsh_dir\$($asset.name)"
if (-not (Test-Path $gsh_dir)) {
	New-Item -ItemType Directory -Path $gsh_dir | Out-Null
}
elseif (Test-Path $gsh_bin) {
	Remove-Item $gsh_bin -Force
}
Write-Host "Downloading " -NoNewline;
Write-Host $asset.name -ForegroundColor Cyan -NoNewline;
Write-Host "..."
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $gsh_bin -UseBasicParsing
$shortcut_path = "$gsh_dir\gsh.exe"
if (Test-Path $shortcut_path) {
	Remove-Item $shortcut_path -Force
}
New-Item -ItemType SymbolicLink -Path "$gsh_dir\gsh.exe" -Target $gsh_bin | Out-Null
Write-Host "Created symbolic link to " -NoNewline;
Write-Host "gsh" -ForegroundColor Cyan -NoNewline;
Write-Host " binary.";

$path = [System.Environment]::GetEnvironmentVariable("PATH", "User")
$gsh_path = [System.IO.Path]::GetDirectoryName($gsh_bin)
if ($path -like "*$gsh_path*") {
	Write-Host "Path already contains " -NoNewline;
	Write-Host $gsh_path -ForegroundColor Cyan -NoNewline;
	Write-Host ".";
}
else {
	$path = $path -split ";" | Where-Object { $_ -ne $gsh_path }
	$path += $gsh_path
	$path = $path -join ";"
	[System.Environment]::SetEnvironmentVariable("PATH", $path, "User")
	Write-Host "Added " -NoNewline;
	Write-Host $gsh_path -ForegroundColor Cyan -NoNewline;
	Write-Host " to PATH."
}
Write-Host "Installation complete!" -ForegroundColor Green;
Write-Host "(?) You may need to restart your terminal or run refreshenv for changes to take effect." -ForegroundColor Yellow
