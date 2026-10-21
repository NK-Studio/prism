<#
.SYNOPSIS
  Free disk space by removing rebuildable Cargo caches under target/.

.DESCRIPTION
  By default removes:
    - every `incremental` cache (target/<profile>/incremental, target/agent-*/<profile>/incremental)
    - whole target/agent-* directories that no build has touched for -AgentIdleHours
      (AGENTS.md: delete the target dirs of finished parallel agents)
  -Profiles debug,fast,release also removes those whole profile directories (full rebuild).

  Never removes target/rea-tools (local analysis output, not a cache) or anything outside
  target/. Refuses to run while cargo or rustc is running, unless -Force.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/clean-target.ps1 -Preview
.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/clean-target.ps1 -Profiles debug
#>
param(
  # List what would be removed, with sizes; remove nothing.
  [switch] $Preview,
  # Remove a target/agent-* directory when no build wrote to it for this many hours.
  [int] $AgentIdleHours = 12,
  # Remove every target/agent-* directory, however recent.
  [switch] $AllAgents,
  # Whole profile directories to remove as well, e.g. debug, fast, release.
  [string[]] $Profiles = @(),
  # Run even while cargo or rustc is running (a build in progress may fail).
  [switch] $Force,
  [string] $TargetDir = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target')
)
$ErrorActionPreference = 'Stop'

# Directories under target/ that are not build caches.
$Keep = @('rea-tools')

function Format-Size([double] $bytes) {
  if ($bytes -ge 1GB) { return '{0:N1} GB' -f ($bytes / 1GB) }
  if ($bytes -ge 1MB) { return '{0:N0} MB' -f ($bytes / 1MB) }
  return '{0:N0} KB' -f ($bytes / 1KB)
}

function Get-Size([string] $path) {
  $sum = (Get-ChildItem -LiteralPath $path -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
  if ($null -eq $sum) { return 0 } else { return [double] $sum }
}

function Get-Free([string] $path) {
  $root = [IO.Path]::GetPathRoot((Resolve-Path -LiteralPath $path).Path)
  return [double] ([IO.DriveInfo]::new($root)).AvailableFreeSpace
}

# Newest write time of a Cargo target directory: its profile folders and their deps/incremental
# entries change on every build (the top folder's own time does not).
function Get-LastBuild([string] $dir) {
  $times = @((Get-Item -LiteralPath $dir).LastWriteTime)
  foreach ($prof in Get-ChildItem -LiteralPath $dir -Directory -Force -ErrorAction SilentlyContinue) {
    $times += $prof.LastWriteTime
    foreach ($sub in 'deps', 'incremental', '.fingerprint', 'build') {
      $p = Join-Path $prof.FullName $sub
      if (Test-Path -LiteralPath $p) { $times += (Get-Item -LiteralPath $p).LastWriteTime }
    }
  }
  return ($times | Measure-Object -Maximum).Maximum
}

if (-not (Test-Path -LiteralPath $TargetDir)) {
  Write-Host "Nothing to clean: $TargetDir does not exist."
  exit 0
}
$TargetDir = (Resolve-Path -LiteralPath $TargetDir).Path
if ((Split-Path $TargetDir -Leaf) -ne 'target') {
  throw "Refusing to clean '$TargetDir': expected a Cargo 'target' directory."
}

$busy = @(Get-Process -Name cargo, rustc -ErrorAction SilentlyContinue)
if ($busy.Count -gt 0 -and -not $Force -and -not $Preview) {
  throw "cargo/rustc is running ($($busy.Count) process(es)). Stop the build first, or pass -Force."
}

$targets = New-Object System.Collections.Generic.List[object]
function Add-Target([string] $path, [string] $why) {
  if ($targets | Where-Object { $path.StartsWith($_.Path + '\') -or $_.Path -eq $path }) { return }
  $targets.Add([pscustomobject] @{ Path = $path; Why = $why })
}

$now = Get-Date
foreach ($dir in Get-ChildItem -LiteralPath $TargetDir -Directory -Force) {
  if ($Keep -contains $dir.Name) { continue }
  if ($dir.Name -like 'agent-*') {
    $last = Get-LastBuild $dir.FullName
    $idle = ($now - $last).TotalHours
    if ($AllAgents -or $idle -ge $AgentIdleHours) {
      Add-Target $dir.FullName ('agent build dir, last build {0:yyyy-MM-dd HH:mm}' -f $last)
      continue
    }
  }
  if ($Profiles -contains $dir.Name) {
    Add-Target $dir.FullName 'profile dir (-Profiles)'
    continue
  }
  # incremental caches of this dir's profiles (target/<profile>/incremental) and, for agent dirs
  # still in use, of their profiles (target/agent-x/<profile>/incremental).
  foreach ($inc in @(Join-Path $dir.FullName 'incremental') + @(Get-ChildItem -LiteralPath $dir.FullName -Directory -Force -ErrorAction SilentlyContinue | ForEach-Object { Join-Path $_.FullName 'incremental' })) {
    if (Test-Path -LiteralPath $inc) { Add-Target $inc 'incremental cache' }
  }
}

if ($targets.Count -eq 0) {
  Write-Host 'Nothing to clean.'
  exit 0
}

$freeBefore = Get-Free $TargetDir
Write-Host ('Free space on the drive: {0}' -f (Format-Size $freeBefore))
Write-Host ''
if ($Preview) {
  $total = 0.0
  foreach ($t in $targets) {
    $size = Get-Size $t.Path
    $total += $size
    Write-Host ('{0,10}  {1}  ({2})' -f (Format-Size $size), $t.Path.Substring($TargetDir.Length - 6), $t.Why)
  }
  Write-Host ''
  Write-Host ('Would free about {0}. Kept: {1}. Run without -Preview to delete.' -f (Format-Size $total), ($Keep -join ', '))
  exit 0
}

$failed = 0
foreach ($t in $targets) {
  Write-Host ('Removing {0}  ({1})' -f $t.Path.Substring($TargetDir.Length - 6), $t.Why)
  # rd handles long paths and deep trees far faster than Remove-Item on Windows PowerShell.
  cmd /c rd /s /q "\\?\$($t.Path)" 2>$null
  if (Test-Path -LiteralPath $t.Path) {
    $failed++
    Write-Warning "Could not remove everything in $($t.Path) (files in use?)."
  }
}
$freeAfter = Get-Free $TargetDir
Write-Host ''
Write-Host ('Freed {0}; free space now {1}.' -f (Format-Size ($freeAfter - $freeBefore)), (Format-Size $freeAfter))
if ($failed -gt 0) { exit 1 }
