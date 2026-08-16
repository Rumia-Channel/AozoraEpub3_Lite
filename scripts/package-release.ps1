param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,

    [Parameter(Mandatory = $true)]
    [ValidateSet("win", "mac", "linux")]
    [string]$Platform,

    [string]$Arch = "x64",

    [string]$Version = "0.1.0",

    [string]$OutputDir = "dist"
)

# Builds the AozoraEpub3_Lite release package with the same layout as the
# original AozoraEpub3: the executable sits next to chuki_*.txt / gaiji /
# presets / template. At run time the binary reads the assets in its own
# directory (see default_config_dir in src/main.rs).

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Test-Path -Path $BinaryPath -PathType Leaf)) {
    throw "Binary not found: $BinaryPath"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$assetDir = Join-Path $repoRoot "assets\aozora"
if (-not (Test-Path -Path (Join-Path $assetDir "chuki_tag.txt") -PathType Leaf)) {
    throw "Aozora assets not found: $assetDir"
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$resolvedBinary = (Resolve-Path -Path $BinaryPath).Path
$resolvedOutputDir = (Resolve-Path -Path $OutputDir).Path
$archiveName = "AozoraEpub3_Lite_{0}_{1}_{2}.zip" -f $Version, $Platform, $Arch
$archivePath = Join-Path $resolvedOutputDir $archiveName
if (Test-Path -Path $archivePath -PathType Leaf) {
    Remove-Item -Path $archivePath -Force
}

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Add-FileToArchive {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.Compression.ZipArchive]$Archive,

        [Parameter(Mandatory = $true)]
        [string]$SourcePath,

        [Parameter(Mandatory = $true)]
        [string]$EntryPath
    )

    [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
        $Archive,
        $SourcePath,
        $EntryPath.Replace('\', '/'),
        [System.IO.Compression.CompressionLevel]::Optimal
    ) | Out-Null
}

function Add-DirectoryToArchive {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.Compression.ZipArchive]$Archive,

        [Parameter(Mandatory = $true)]
        [string]$SourceDir,

        [Parameter(Mandatory = $true)]
        [string]$EntryRoot
    )

    $resolvedDir = (Resolve-Path -Path $SourceDir).Path
    $resolvedDirWithSep = $resolvedDir.TrimEnd('\') + '\'
    $files = Get-ChildItem -Path $resolvedDir -Recurse -File
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($resolvedDirWithSep.Length)
        $entryPath = Join-Path -Path $EntryRoot -ChildPath $relative
        Add-FileToArchive -Archive $Archive -SourcePath $file.FullName -EntryPath $entryPath
    }
}

$archive = [System.IO.Compression.ZipFile]::Open(
    $archivePath,
    [System.IO.Compression.ZipArchiveMode]::Create
)

try {
    # Executable at the root (the original keeps AozoraEpub3.jar there).
    $binaryName = [System.IO.Path]::GetFileName($resolvedBinary)
    Add-FileToArchive -Archive $archive -SourcePath $resolvedBinary -EntryPath $binaryName

    # Note assets and character replacement at the root, like the original.
    foreach ($file in Get-ChildItem -Path $assetDir -File) {
        if ($file.Extension -eq ".ini") {
            continue
        }
        Add-FileToArchive -Archive $archive -SourcePath $file.FullName -EntryPath $file.Name
    }

    # Device presets go under presets/, like the original.
    foreach ($file in Get-ChildItem -Path $assetDir -Filter "*.ini" -File) {
        Add-FileToArchive -Archive $archive -SourcePath $file.FullName -EntryPath ("presets/" + $file.Name)
    }

    # Gaiji fonts and the EPUB template.
    Add-DirectoryToArchive -Archive $archive -SourceDir (Join-Path $assetDir "gaiji") -EntryRoot "gaiji"
    Add-DirectoryToArchive -Archive $archive -SourceDir (Join-Path $assetDir "template") -EntryRoot "template"

    # Documentation.
    foreach ($doc in @("LICENSE.txt", "README.md")) {
        $docPath = Join-Path $repoRoot $doc
        if (Test-Path -Path $docPath -PathType Leaf) {
            Add-FileToArchive -Archive $archive -SourcePath $docPath -EntryPath $doc
        }
    }
} finally {
    $archive.Dispose()
}

Write-Output $archivePath
