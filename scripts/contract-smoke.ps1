# End-to-end check of the sf-cli v2 JSON contract (docs/cli-contract.md).
# Builds synthetic disks, runs every v2 command (and its error paths), and
# checks that stdout is pure NDJSON with the documented events, fields,
# types and exit codes. With -Samples, also saves each run's real stdout to
# docs/contract-samples/ for the GUI's parser tests.
#
#   powershell -ExecutionPolicy Bypass -File scripts\contract-smoke.ps1 [-Samples] [-Release]
#
# Needs: the NTFS fixture (crates/sf-core/tests/fixtures/ntfs-small.vhd.gz).
# Runs unelevated; nothing touches a real drive.

param([switch]$Samples, [switch]$Release)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$env:Path += ";$env:USERPROFILE\.cargo\bin"

$profileDir = if ($Release) { 'release' } else { 'debug' }
Push-Location $root
try {
    if ($Release) { cargo build -q --release -p sf-cli } else { cargo build -q -p sf-cli }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
} finally { Pop-Location }
$cli = Join-Path $targetDir "$profileDir\sf-cli.exe"

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-smoke-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory $work | Out-Null
$sampleDir = Join-Path $root 'docs\contract-samples'
if ($Samples) { New-Item -ItemType Directory -Force $sampleDir | Out-Null }

$script:failures = 0
$script:checks = 0
function Fail([string]$msg) { $script:failures++; Write-Host "  FAIL: $msg" -ForegroundColor Red }
function Check([bool]$cond, [string]$msg) { $script:checks++; if (-not $cond) { Fail $msg } }

# Runs sf-cli in $work (on the system drive, so the same-drive refusals
# trigger; relative paths keep local paths out of the samples).
function Run([string[]]$cliArgs, [int]$expectExit, [string]$sample) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $cli
    $psi.Arguments = ($cliArgs | ForEach-Object { if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ } }) -join ' '
    $psi.WorkingDirectory = $work
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $p = [System.Diagnostics.Process]::Start($psi)
    $stderrTask = $p.StandardError.ReadToEndAsync()
    $stdout = $p.StandardOutput.ReadToEnd()
    $p.WaitForExit()
    $stderr = $stderrTask.Result
    Write-Host ("sf-cli " + $psi.Arguments + "  -> exit " + $p.ExitCode)
    Check ($p.ExitCode -eq $expectExit) "expected exit $expectExit, got $($p.ExitCode). stderr: $stderr"
    if ($Samples -and $sample) { [System.IO.File]::WriteAllText((Join-Path $sampleDir "$sample.ndjson"), $stdout, (New-Object System.Text.UTF8Encoding $false)) }

    $events = @()
    if ($stdout.Length -gt 0) {
        Check ($stdout.EndsWith("`n")) "stdout must end with a newline"
        foreach ($line in $stdout.TrimEnd("`n").Split("`n")) {
            try { $o = $line | ConvertFrom-Json } catch { Fail "not JSON: $line"; continue }
            Check ($o.event -is [string]) "every line needs a string 'event': $line"
            $events += , $o
        }
    }
    if ($expectExit -eq 2) { Check ($stdout.Length -eq 0) "usage errors must print no JSON (got: $stdout)" }
    if ($expectExit -eq 1) {
        Check ($events.Count -ge 1 -and $events[-1].event -eq 'error') "runtime errors must end with an error event"
        if ($events.Count -ge 1) { Check ($events[-1].message -is [string] -and $events[-1].message.Length -gt 0) "error needs a message" }
    }
    return , $events
}

function IsInt($v) { $v -is [int] -or $v -is [long] -or $v -is [decimal] }
# Checks the fields of one event: 'int', 'string', 'bool', 'array', with '?' for nullable.
function Shape($ev, [hashtable]$fields) {
    foreach ($k in $fields.Keys) {
        $t = $fields[$k]; $nullable = $t.EndsWith('?'); $t = $t.TrimEnd('?')
        $has = $ev.PSObject.Properties.Name -contains $k
        Check $has "$($ev.event): missing field '$k'"
        if (-not $has) { continue }
        $v = $ev.$k
        if ($null -eq $v) { Check $nullable "$($ev.event).$k must not be null"; continue }
        $ok = switch ($t) {
            'int' { IsInt $v } 'string' { $v -is [string] } 'bool' { $v -is [bool] } 'array' { $v -is [array] }
        }
        Check $ok "$($ev.event).$k should be $t, got '$v' ($($v.GetType().Name))"
    }
}
function Events($evs, [string]$name) { @($evs | Where-Object { $_.event -eq $name }) }
function OneOf($v, [string[]]$allowed, [string]$what) { Check ($allowed -contains $v) "$what '$v' not in: $($allowed -join ', ')" }

# ---- synthetic carve disk: real files from Windows' own encoders --------------
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
function Image-Bytes([System.Drawing.Imaging.ImageFormat]$fmt) {
    $bmp = New-Object System.Drawing.Bitmap 96, 64
    for ($x = 0; $x -lt 96; $x++) { for ($y = 0; $y -lt 64; $y++) { $bmp.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(($x * 3) % 256, ($y * 4) % 256, ($x * $y) % 256)) } }
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, $fmt); $bmp.Dispose()
    , $ms.ToArray()
}
function Zip-Bytes([hashtable]$entries) {
    $ms = New-Object System.IO.MemoryStream
    $zip = New-Object System.IO.Compression.ZipArchive($ms, [System.IO.Compression.ZipArchiveMode]::Create, $true)
    foreach ($k in ($entries.Keys | Sort-Object)) {
        $w = New-Object System.IO.StreamWriter($zip.CreateEntry($k).Open()); $w.Write($entries[$k]); $w.Close()
    }
    $zip.Dispose()
    , $ms.ToArray()
}
$pdfText = "%PDF-1.4`n1 0 obj << /Type /Catalog >> endobj`nxref`n0 1`ntrailer << /Root 1 0 R >>`nstartxref`n9`n%%EOF`n"
$carveFiles = @(
    @{ type = 'jpg'; offset = 0x10000; bytes = (Image-Bytes ([System.Drawing.Imaging.ImageFormat]::Jpeg)) },
    @{ type = 'png'; offset = 0x40000; bytes = (Image-Bytes ([System.Drawing.Imaging.ImageFormat]::Png)) },
    @{ type = 'gif'; offset = 0x80000; bytes = (Image-Bytes ([System.Drawing.Imaging.ImageFormat]::Gif)) },
    @{ type = 'zip'; offset = 0xC0000; bytes = (Zip-Bytes @{ 'readme.txt' = 'hello'; 'data/numbers.csv' = ((1..500) -join ',') }) },
    @{ type = 'docx'; offset = 0x100000; bytes = (Zip-Bytes @{ '[Content_Types].xml' = '<Types/>'; 'word/document.xml' = '<w:document/>' }) },
    @{ type = 'pdf'; offset = 0x140000; bytes = [System.Text.Encoding]::ASCII.GetBytes($pdfText) }
)
$diskSize = 0x200000
$disk = New-Object byte[] $diskSize
for ($i = 0; $i -lt $diskSize; $i++) { $disk[$i] = 0x5A }
foreach ($f in $carveFiles) { [Array]::Copy($f.bytes, 0, $disk, $f.offset, $f.bytes.Length) }
[System.IO.File]::WriteAllBytes((Join-Path $work 'carve-disk.img'), $disk)

# A map that marks one sector inside the PNG as bad.
$bad = 0x40000 + 512
$map = "# Mapfile`n0x00000000     +`n" + ("0x{0:X8}  0x{1:X8}  +`n0x{2:X8}  0x{3:X8}  -`n0x{4:X8}  0x{5:X8}  +`n" -f 0, $bad, $bad, 512, ($bad + 512), ($diskSize - $bad - 512))
[System.IO.File]::WriteAllText((Join-Path $work 'carve-disk.map'), $map)
[System.IO.File]::WriteAllText((Join-Path $work 'wrong-size.map'), "# Mapfile`n0x0     +`n0x00000000  0x00001000  +`n")

# ---- NTFS fixture ---------------------------------------------------------------
$fixture = Join-Path $root 'crates\sf-core\tests\fixtures\ntfs-small.vhd.gz'
$in = [System.IO.File]::OpenRead($fixture)
$gz = New-Object System.IO.Compression.GZipStream($in, [System.IO.Compression.CompressionMode]::Decompress)
$outF = [System.IO.File]::Create((Join-Path $work 'ntfs.vhd'))
$gz.CopyTo($outF); $outF.Close(); $gz.Close()

$startShape = @{ source = 'string'; mode = 'string'; size = 'int'; sector_size = 'int' }

Write-Host "`n== version"
$ev = Run @('version', '--json') 0 'version'
Check ($ev.Count -eq 1) "version prints exactly one line"
Shape $ev[0] @{ version = 'string'; contract = 'int' }
Check ($ev[0].contract -eq 2) "contract must be 2"

Write-Host "`n== devices"
$ev = Run @('devices', '--json') 0 $null
Check ($ev.Count -eq 1) "devices prints exactly one line"
Shape $ev[0] @{ elevated = 'bool'; devices = 'array' }
foreach ($d in $ev[0].devices) {
    Shape $d @{ path = 'string'; model = 'string?'; serial = 'string?'; size = 'int?'; sector_size = 'int?'; bus = 'string'; removable = 'bool'; system = 'bool' }
    OneOf $d.bus @('sata', 'nvme', 'usb', 'scsi', 'sd', 'virtual', 'other') 'bus'
}
Check (@($ev[0].devices | Where-Object { $_.system }).Count -ge 1) "the drive holding Windows should be marked system"
$systemDrive = @($ev[0].devices | Where-Object { $_.system })[0]
if ($Samples) {
    # Real shape, but don't publish this machine's serial numbers.
    $red = $ev[0]; foreach ($d in $red.devices) { if ($d.serial) { $d.serial = 'REDACTED' } }
    [System.IO.File]::WriteAllText((Join-Path $sampleDir 'devices.ndjson'), ($red | ConvertTo-Json -Depth 5 -Compress) + "`n", (New-Object System.Text.UTF8Encoding $false))
}

Write-Host "`n== image: refuses outputs on the source drive (device mode, unelevated is fine)"
$ev = Run @('image', '--source', $systemDrive.path, '--dest', 'x.img', '--map', 'x.map', '--json') 1 'error_image_same_drive'
Check ($ev[-1].message -match 'source drive') "same-drive refusal message"
Check (-not (Test-Path (Join-Path $work 'x.map'))) "refusal must happen before anything is created"

Write-Host "`n== carve --list-types"
$ev = Run @('carve', '--list-types', '--json') 0 'carve_list_types'
Shape $ev[0] @{ types = 'array' }
foreach ($t in $ev[0].types) { Shape $t @{ id = 'string'; description = 'string' } }
$typeIds = $ev[0].types | ForEach-Object { $_.id }
foreach ($t in 'jpg', 'png', 'gif', 'pdf', 'zip', 'docx', 'xlsx', 'pptx', 'mp4', 'mov', 'sqlite', '7z', 'rar') { Check ($typeIds -contains $t) "type $t listed" }

Write-Host "`n== carve"
$ev = Run @('carve', '--source', 'carve-disk.img', '--out', 'carved', '--json') 0 'carve'
Check ($ev[0].event -eq 'start') "carve starts with start"
Shape $ev[0] $startShape
Check ($ev[0].mode -eq 'file') "file mode"
foreach ($p in (Events $ev 'carve_progress')) { Shape $p @{ bytes_scanned = 'int'; bytes_total = 'int'; files_found = 'int' } }
$found = @(Events $ev 'file')
foreach ($f in $found) { Shape $f @{ offset = 'int'; size = 'int'; type = 'string'; name = 'string'; complete = 'bool' } }
Check ($found.Count -eq $carveFiles.Count) "carve should find $($carveFiles.Count) files, found $($found.Count)"
foreach ($cf in $carveFiles) {
    $hit = $found | Where-Object { $_.offset -eq $cf.offset }
    if (-not $hit) { Fail "no carved file at offset $($cf.offset) ($($cf.type))"; continue }
    Check ($hit.type -eq $cf.type) "type at $($cf.offset): expected $($cf.type), got $($hit.type)"
    Check ($hit.complete) "$($cf.type) should be complete"
    Check ($hit.name -eq ('{0}/f{1:D7}.{0}' -f $cf.type, ($cf.offset / 512))) "name format: $($hit.name)"
    $got = [System.IO.File]::ReadAllBytes((Join-Path (Join-Path $work 'carved') $hit.name))
    Check ([Convert]::ToBase64String($got) -eq [Convert]::ToBase64String($cf.bytes)) "$($cf.type) carved bytes must match the original exactly"
}
$done = $ev[-1]
Check ($done.event -eq 'done') "carve ends with done"
Shape $done @{ files_found = 'int'; bytes_written = 'int' }

Write-Host "`n== carve --map (bad sector inside the PNG) --types png,jpg --start/--end"
$ev = Run @('carve', '--source', 'carve-disk.img', '--out', 'carved-map', '--map', 'carve-disk.map', '--types', 'png,jpg', '--start', '4096', '--end', '786432', '--json') 0 'carve_map'
$found = @(Events $ev 'file')
Check (($found | ForEach-Object { $_.type }) -join ',' -eq 'jpg,png') "types filter + range: got $(($found | ForEach-Object { $_.type }) -join ',')"
$png = $found | Where-Object { $_.type -eq 'png' }
Check ($png -and -not $png.complete) "PNG overlapping a bad map range must be complete:false"

Write-Host "`n== carve error paths"
$ev = Run @('carve', '--source', 'carve-disk.img', '--out', 'c2', '--types', 'exe', '--json') 1 'error_carve_unknown_type'
$ev = Run @('carve', '--source', 'carve-disk.img', '--out', 'c3', '--start', '9000', '--end', '100', '--json') 1 $null
$ev = Run @('carve', '--source', 'carve-disk.img', '--out', 'c4', '--map', 'wrong-size.map', '--json') 1 'error_carve_wrong_map'
Check ($ev[-1].message -match 'Wrong map') "wrong-size map message"
$ev = Run @('carve', '--source', 'no-such.img', '--out', 'c5', '--json') 1 $null
$ev = Run @('carve', '--source', 'carve-disk.img', '--json') 2 $null

Write-Host "`n== scan (to find the fixture's partition)"
$ev = Run @('scan', '--source', 'ntfs.vhd', '--json') 0 $null
$lba = ($ev[-1].partitions | Where-Object { $_.filesystem -eq 'ntfs' })[0].start_lba
Check ($null -ne $lba) "fixture has an NTFS partition"

Write-Host "`n== files"
$ev = Run @('files', '--source', 'ntfs.vhd', '--partition-lba', "$lba", '--json') 0 'files'
Check ($ev[0].event -eq 'start') "files starts with start"
Shape $ev[0] ($startShape + @{ partition_lba = 'int'; filesystem = 'string' })
Check ($ev[0].filesystem -eq 'ntfs') "filesystem ntfs"
$prog = @(Events $ev 'files_progress')
Check ($prog.Count -ge 1) "files_progress emitted"
foreach ($p in $prog) { Shape $p @{ records_read = 'int'; records_total = 'int' } }
$files = @(Events $ev 'file')
foreach ($f in $files) {
    Shape $f @{ id = 'int'; parent_id = 'int'; name = 'string'; path = 'string?'; is_dir = 'bool'; size = 'int'; deleted = 'bool'; modified = 'string?'; created = 'string?'; recoverable = 'string' }
    OneOf $f.recoverable @('full', 'partial', 'none') 'recoverable'
    if ($f.modified) { Check ($f.modified -match '^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$') "RFC 3339 UTC: $($f.modified)" }
    if ($f.path) { Check ($f.path.StartsWith('/') -and $f.path.EndsWith('/' + $f.name)) "path ends with name: $($f.path)" }
}
$done = $ev[-1]
Shape $done @{ files = 'int'; deleted = 'int' }
Check ($done.files -eq $files.Count) "done.files matches file events"
Check ($done.deleted -eq @($files | Where-Object { $_.deleted }).Count) "done.deleted matches"
$byPath = @{}; foreach ($f in $files) { if ($f.path) { $byPath[$f.path] = $f } }
Check ($byPath['/Pictures/deleted_photo.jpg'].deleted) "deleted photo listed as deleted"
Check ($byPath['/OldStuff/inner/memo.txt'].deleted) "file in deleted folder keeps its path"
Check (-not ($files | Where-Object { $_.name -eq '$MFT' })) "NTFS metadata files are not listed"

$ev = Run @('files', '--source', 'ntfs.vhd', '--partition-lba', "$lba", '--deleted-only', '--json') 0 'files_deleted_only'
Check (@(Events $ev 'file' | Where-Object { -not $_.deleted }).Count -eq 0) "--deleted-only lists only deleted entries"
Check ((Events $ev 'file').Count -ge 4) "--deleted-only finds the deleted fixture entries"

$ev = Run @('files', '--source', 'carve-disk.img', '--partition-lba', '0', '--json') 1 'error_files_not_ntfs'
Check ($ev[-1].message -match 'NTFS only') "non-NTFS message"
$ev = Run @('files', '--source', 'ntfs.vhd', '--partition-lba', '999999999', '--json') 1 $null

Write-Host "`n== extract"
$photo = $byPath['/Pictures/deleted_photo.jpg'].id
$docs = $byPath['/Documents'].id
$ev = Run @('extract', '--source', 'ntfs.vhd', '--partition-lba', "$lba", '--ids', "$photo,$docs,4000000", '--out', 'restored', '--json') 0 'extract'
Shape $ev[0] ($startShape + @{ partition_lba = 'int'; filesystem = 'string' })
foreach ($p in (Events $ev 'extract_progress')) { Shape $p @{ files_done = 'int'; files_total = 'int'; bytes_done = 'int'; bytes_total = 'int' } }
$items = @(Events $ev 'extracted')
foreach ($i in $items) { Shape $i @{ id = 'int'; path = 'string?'; size = 'int'; complete = 'bool'; note = 'string?' } }
$pi = $items | Where-Object { $_.id -eq $photo }
Check ($pi.path -eq 'Pictures/deleted_photo.jpg' -and $pi.complete) "deleted photo extracted to its folder"
Check (Test-Path (Join-Path $work 'restored\Documents\notes\deep\nested.txt')) "directory id extracts recursively"
$missing = $items | Where-Object { $_.id -eq 4000000 }
Check ($missing -and $null -eq $missing.path -and -not $missing.complete -and $missing.note) "unknown id reported as a failed item"
Shape $ev[-1] @{ extracted = 'int'; failed = 'int' }
Check ($ev[-1].failed -eq 1) "one failure (the unknown id)"

$compressed = $byPath['/compressed.txt'].id
"$compressed`n`n$photo`n" | Set-Content -Encoding ascii (Join-Path $work 'ids.txt')
$ev = Run @('extract', '--source', 'ntfs.vhd', '--partition-lba', "$lba", '--ids-file', 'ids.txt', '--out', 'restored2', '--json') 0 'extract_ids_file'
$ci = @(Events $ev 'extracted') | Where-Object { $_.id -eq $compressed }
Check ($ci -and -not $ci.complete -and $ci.note -match 'compressed') "compressed file flagged complete:false with a note"

$ev = Run @('extract', '--source', 'ntfs.vhd', '--partition-lba', "$lba", '--ids', '1', '--ids-file', 'ids.txt', '--out', 'r3', '--json') 2 $null
'12x' | Set-Content -Encoding ascii (Join-Path $work 'bad-ids.txt')
$ev = Run @('extract', '--source', 'ntfs.vhd', '--partition-lba', "$lba", '--ids-file', 'bad-ids.txt', '--out', 'r4', '--json') 1 $null
$ev = Run @('extract', '--source', $systemDrive.path, '--partition-lba', '2048', '--ids', '5', '--out', 'r5', '--json') 1 $null
Check ($ev[-1].message -match 'source drive') "extract refuses --out on the source drive"

Remove-Item $work -Recurse -Force
Write-Host ""
if ($script:failures -gt 0) {
    Write-Host "$script:failures of $script:checks checks FAILED" -ForegroundColor Red
    exit 1
}
Write-Host "all $script:checks checks passed" -ForegroundColor Green
