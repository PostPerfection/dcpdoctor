param(
    [Parameter(Mandatory)] [string] $Binary,
    [Parameter(Mandatory)] [string[]] $SearchDirectories
)

$ErrorActionPreference = "Stop"

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$dumpbin = & $vswhere -latest -products * -find "VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe" | Select-Object -First 1
if (-not $dumpbin) { throw "vswhere found no dumpbin.exe" }

# the exe has to start on a machine without the VC++ redistributable
$visualStudio = & $vswhere -latest -products * -property installationPath | Select-Object -First 1
if (-not $visualStudio) { throw "vswhere found no Visual Studio installation" }
$redistributableVersion = (Get-Content (Join-Path $visualStudio "VC\Auxiliary\Build\Microsoft.VCRedistVersion.default.txt") -TotalCount 1).Trim()
$runtimeDirectories = @(Get-ChildItem -Directory -Path (Join-Path $visualStudio "VC\Redist\MSVC\$redistributableVersion\x64") -Filter "Microsoft.VC*.CRT")
if ($runtimeDirectories.Count -ne 1) { throw "expected one Microsoft.VC*.CRT directory for redistributable $redistributableVersion, found $($runtimeDirectories.Count)" }
$SearchDirectories = @($runtimeDirectories[0].FullName) + $SearchDirectories

$systemDirectory = Join-Path $env:SystemRoot "System32"
$binaryPath = (Resolve-Path $Binary).Path
$distributionDirectory = Split-Path -Parent $binaryPath

$queue = [System.Collections.Generic.Queue[string]]::new()
$queue.Enqueue($binaryPath)
while ($queue.Count -gt 0) {
    $file = $queue.Dequeue()
    $output = & $dumpbin /nologo /dependents $file
    if ($LASTEXITCODE -ne 0) { throw "dumpbin /dependents failed on $file" }
    $dependents = $output | Where-Object { $_ -match '^\s+\S+\.dll\s*$' } | ForEach-Object { $_.Trim() }
    foreach ($dll in $dependents) {
        if (Test-Path (Join-Path $distributionDirectory $dll)) { continue }
        $source = $SearchDirectories | ForEach-Object { Join-Path $_ $dll } | Where-Object { Test-Path $_ } | Select-Object -First 1
        if ($source) {
            Copy-Item $source $distributionDirectory
            Write-Output "$file imports $dll, copied from $source"
            $queue.Enqueue((Join-Path $distributionDirectory $dll))
            continue
        }
        # api-ms-win and ext-ms-win are api set names the loader maps, never files
        if ($dll -match '^(api|ext)-ms-' -or (Test-Path (Join-Path $systemDirectory $dll))) { continue }
        throw "$file imports $dll, which is in neither System32 nor $($SearchDirectories -join ', ')"
    }
}
