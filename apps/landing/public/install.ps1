$ErrorActionPreference = "Stop"

# Seashail installer (source-build).
#
# This script clones the repo and builds the `seashail` binary with Cargo.
# It intentionally does NOT download a prebuilt binary.
#
# Usage:
#   irm https://seashail.com/install.ps1 | iex
#
# Environment overrides:
#   SEASHAIL_REPO           (default: https://github.com/seashail/seashail)
#   SEASHAIL_REF            (default: main)       # branch/tag/commit
#   SEASHAIL_VERSION        (alias for SEASHAIL_REF)
#   SEASHAIL_INSTALL_DIR    (default: $HOME\.local\bin)

function Msg([string]$Text) { Write-Host $Text }
function Die([string]$Text) { Write-Error ("seashail install: " + $Text); exit 1 }

function Need-Cmd([string]$Name) {
  $cmd = Get-Command $Name -ErrorAction SilentlyContinue
  if (-not $cmd) { Die ("missing dependency: " + $Name) }
}

Need-Cmd git
Need-Cmd cargo

$repo = if ($env:SEASHAIL_REPO) { $env:SEASHAIL_REPO } else { "https://github.com/seashail/seashail" }
$ref = if ($env:SEASHAIL_VERSION) { $env:SEASHAIL_VERSION } elseif ($env:SEASHAIL_REF) { $env:SEASHAIL_REF } else { "main" }
$installDir = if ($env:SEASHAIL_INSTALL_DIR) { $env:SEASHAIL_INSTALL_DIR } else { Join-Path $HOME ".local\bin" }
$binName = "seashail.exe"

$tmpRoot = [System.IO.Path]::GetTempPath()
$tmp = Join-Path $tmpRoot ("seashail-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
  $repoDir = Join-Path $tmp "seashail"

  Msg ("Cloning " + $repo + " (" + $ref + ")...")
  git clone --quiet --depth 1 --branch $ref $repo $repoDir | Out-Null
  if ($LASTEXITCODE -ne 0) { Die "git clone failed (try: `$env:SEASHAIL_REF='main'`)" }

  Msg "Building (release)..."
  Push-Location $repoDir
  try {
    cargo build -p seashail --release
  } finally {
    Pop-Location
  }

  New-Item -ItemType Directory -Force -Path $installDir | Out-Null
  $src = Join-Path $repoDir (Join-Path "target" (Join-Path "release" $binName))
  $dst = Join-Path $installDir $binName
  Copy-Item -Force $src $dst

  Msg ("Installed: " + $dst)

  $pathParts = ($env:Path -split ";") | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne "" }
  if (-not ($pathParts -contains $installDir)) {
    Msg "Add to PATH (example, current session):"
    Msg ("  `$env:Path = `\"" + $installDir + ";`$env:Path`\"")
    Msg ("To persist, add " + $installDir + " to your User PATH in Windows settings.")
  }

  Msg "Run MCP (stdio):"
  Msg "  seashail mcp"
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
