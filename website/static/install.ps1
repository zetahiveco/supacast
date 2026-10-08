# Supacast installer — Windows
#
# Usage: powershell -c "irm https://supacast.vercel.app/install.ps1 | iex"
$ErrorActionPreference = "Stop"

$repo = "zetahiveco/supacast"
$appName = "Supacast"

function Fail($message) {
    Write-Host "Error: $message" -ForegroundColor Red
    exit 1
}

if (-not $IsWindows -and $env:OS -ne "Windows_NT") {
    Fail "This script is for Windows. On macOS/Linux use: curl -fsSL https://supacast.vercel.app/install.sh | bash"
}

Write-Host "==> Fetching the latest $appName release..." -ForegroundColor Cyan
try {
    $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
} catch {
    Fail "Could not reach GitHub Releases. Check your network connection."
}

$asset = $release.assets |
    Where-Object { $_.name -match "x64-setup\.exe$|x64_en-US\.msi$" } |
    Select-Object -First 1

if (-not $asset) {
    Fail "No Windows installer found in the latest release."
}

$tmp = Join-Path $env:TEMP $asset.name
Write-Host "==> Downloading $($asset.name)..." -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmp -UseBasicParsing
} catch {
    Fail "Download failed."
}

Write-Host "==> Running installer (silent)..." -ForegroundColor Cyan
# Tauri NSIS installers accept /S for a silent install.
try {
    Start-Process -FilePath $tmp -ArgumentList "/S" -Wait
} catch {
    Fail "Installer failed to run: $_"
}
Remove-Item $tmp -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "Supacast installed successfully!" -ForegroundColor Green
Write-Host ""
Write-Host "  Launch it from the Start Menu or the system-tray rocket icon."
Write-Host "  It will now start automatically every time you log in."
Write-Host ""
Write-Host "  Docs: https://supacast.vercel.app/docs/"
Write-Host "  Made by https://github.com/harishdeivanayagam - https://zetahive.co"