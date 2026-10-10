# Supacast installer — Windows
#
# Usage: powershell -c "irm https://supacast-omega.vercel.app/install.ps1 | iex"
$ErrorActionPreference = "Stop"

$repo = "zetahiveco/supacast"
$appName = "Supacast"

function Fail($message) {
    Write-Host "Error: $message" -ForegroundColor Red
    exit 1
}

if (-not $IsWindows -and $env:OS -ne "Windows_NT") {
    Fail "This script is for Windows. On macOS/Linux use: curl -fsSL https://supacast-omega.vercel.app/install.sh | bash"
}

Write-Host "==> Fetching the latest $appName release..." -ForegroundColor Cyan
try {
    $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
} catch {
    Fail "Could not reach GitHub Releases. Check your network connection."
}

$asset = $release.assets |
    Where-Object { $_.name -match "x86_64-pc-windows-msvc\.zip$" } |
    Select-Object -First 1

if (-not $asset) {
    Fail "No Windows release found in the latest release."
}

$tmp = Join-Path $env:TEMP $asset.name
Write-Host "==> Downloading $($asset.name)..." -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmp -UseBasicParsing
} catch {
    Fail "Download failed."
}

# Install destination: a per-user directory on the PATH.
$destDir = Join-Path $env:LOCALAPPDATA "Programs\Supacast"
Write-Host "==> Installing to $destDir..." -ForegroundColor Cyan
if (-not (Test-Path $destDir)) {
    New-Item -ItemType Directory -Path $destDir -Force | Out-Null
}

try {
    Expand-Archive -Path $tmp -DestinationPath $destDir -Force
} catch {
    Fail "Extraction failed: $_"
}
Remove-Item $tmp -ErrorAction SilentlyContinue

# Add the install directory to the user PATH if it isn't there yet.
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$destDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$destDir", "User")
    Write-Host "==> Added $destDir to your user PATH (restart your terminal to use 'supacast')." -ForegroundColor Cyan
}

# Register launch-at-login via the registry Run key.
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$exePath = Join-Path $destDir "supacast.exe"
try {
    Set-ItemProperty -Path $runKey -Name "Supacast" -Value "`"$exePath`""
    Write-Host "==> Supacast will start automatically when Windows boots." -ForegroundColor Cyan
} catch {
    Write-Host "Could not register launch-at-login (you can run Supacast manually)." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "Supacast installed successfully!" -ForegroundColor Green
Write-Host ""
Write-Host "  Run 'supacast' to start it - look for the system-tray rocket icon."
Write-Host "  Launch it once so it registers the global shortcut."
Write-Host ""
Write-Host "  Docs: https://supacast-omega.vercel.app/docs/"
Write-Host "  Made by https://github.com/harishdeivanayagam - https://zetahive.co"
