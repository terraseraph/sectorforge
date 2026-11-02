#Requires -RunAsAdministrator
# Builds ntfs-small.vhd.gz: a 16 MiB fixed VHD (MBR, one NTFS partition)
# formatted by Windows' own NTFS driver, with files created and then deleted,
# so sf-core's NTFS reader is tested against real on-disk structures rather
# than only hand-built ones. Also writes ntfs-small.manifest.json with the
# SHA-256 of every file as written (deleted ones hashed before deletion).
#
# Run from an elevated PowerShell:
#   powershell -ExecutionPolicy Bypass -File make-ntfs-fixture.ps1
# Needs: Windows 10/11 (diskpart + Storage cmdlets). Touches only the VHD it
# creates; the drive letter is picked from the free ones.

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$work = Join-Path $env:TEMP "sf-ntfs-fixture"
$vhd = Join-Path $work "ntfs-small.vhd"
# A previous failed run can leave the VHD attached, which locks the file.
if (Test-Path $vhd) { Dismount-DiskImage -ImagePath $vhd -ErrorAction SilentlyContinue | Out-Null }
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory $work | Out-Null

"create vdisk file=`"$vhd`" maximum=16 type=fixed`nexit" | Set-Content -Encoding ascii (Join-Path $work "create.txt")
diskpart /s (Join-Path $work "create.txt") | Out-Null
Mount-DiskImage -ImagePath $vhd | Out-Null
try {
$disk = Get-DiskImage -ImagePath $vhd | Get-Disk
Initialize-Disk -Number $disk.Number -PartitionStyle MBR
$part = New-Partition -DiskNumber $disk.Number -UseMaximumSize -AssignDriveLetter
Format-Volume -Partition $part -FileSystem NTFS -NewFileSystemLabel SFTEST -Confirm:$false | Out-Null
$root = "$($part.DriveLetter):\"

# Compressible, unique, deterministic content: "<tag> line N" repeated.
function Make-Text([string]$tag, [int]$bytes) {
    $sb = New-Object System.Text.StringBuilder
    $i = 0
    while ($sb.Length -lt $bytes) { [void]$sb.Append("$tag line $i`n"); $i++ }
    [System.Text.Encoding]::ASCII.GetBytes($sb.ToString().Substring(0, $bytes))
}
function Put([string]$rel, [byte[]]$data) {
    $p = Join-Path $root $rel
    # New-Item rejects a drive root ("D:\") in Windows PowerShell 5.1; CreateDirectory accepts it.
    [System.IO.Directory]::CreateDirectory((Split-Path -Parent $p)) | Out-Null
    [System.IO.File]::WriteAllBytes($p, $data)
}

$manifest = @()
function Record([string]$rel, [bool]$deleted, [string]$note) {
    $p = Join-Path $root $rel
    $h = (Get-FileHash -Algorithm SHA256 $p).Hash.ToLower()
    $script:manifest += [ordered]@{ path = "/" + $rel.Replace('\', '/'); size = (Get-Item $p).Length; sha256 = $h; deleted = $deleted; note = $note }
}

Put "Documents\report.txt" (Make-Text "report" 300)                  # resident $DATA
Put "Documents\notes\deep\nested.txt" (Make-Text "nested" 5000)       # non-resident
Put "Pictures\photo.jpg" (Make-Text "photo" 200000)
Put "Pictures\deleted_photo.jpg" (Make-Text "deleted-photo" 150000)
Put "OldStuff\letter.txt" (Make-Text "letter" 9000)
Put "OldStuff\inner\memo.txt" (Make-Text "memo" 700)
Put "caf$([char]0xE9) $([char]0xFC)n$([char]0xEF)code.txt" (Make-Text "unicode" 1234)
Put "A Very Long File Name That Needs A Short Alias.txt" (Make-Text "longname" 2100)

# Fragmented: alternate small appends between two files, closing each time.
$fa = Join-Path $root "fragmented.bin"; $fb = Join-Path $root "interleave.bin"
for ($i = 0; $i -lt 40; $i++) {
    $a = [System.IO.File]::Open($fa, 'Append'); $d = Make-Text "frag$i" 8192; $a.Write($d, 0, $d.Length); $a.Close()
    $b = [System.IO.File]::Open($fb, 'Append'); $d = Make-Text "inter$i" 8192; $b.Write($d, 0, $d.Length); $b.Close()
}

# Many long hard links overflow the 1 KiB MFT record -> $ATTRIBUTE_LIST.
Put "Links\target.txt" (Make-Text "hardlinked" 4000)
for ($i = 0; $i -lt 40; $i++) {
    fsutil hardlink create (Join-Path $root ("Links\link_with_a_fairly_long_name_to_fill_the_record_{0:D3}.txt" -f $i)) (Join-Path $root "Links\target.txt") | Out-Null
}

# Sparse: 1 MiB logical, data only at the start and end.
$sp = Join-Path $root "sparse.bin"
[System.IO.File]::WriteAllBytes($sp, (New-Object byte[] 1048576))
fsutil sparse setflag $sp | Out-Null
fsutil sparse setrange $sp 0 1048576 | Out-Null
$s = [System.IO.File]::Open($sp, 'Open'); $d = Make-Text "sparse-head" 4096; $s.Write($d, 0, $d.Length)
$s.Seek(1048576 - 4096, 'Begin') | Out-Null; $d = Make-Text "sparse-tail" 4096; $s.Write($d, 0, $d.Length); $s.Close()

# NTFS-compressed: sf-core must flag this complete:false rather than emit garbage.
Put "compressed.txt" (Make-Text "compressed" 65536)
compact /c (Join-Path $root "compressed.txt") | Out-Null

foreach ($f in "Documents\report.txt", "Documents\notes\deep\nested.txt", "Pictures\photo.jpg",
    "caf$([char]0xE9) $([char]0xFC)n$([char]0xEF)code.txt", "A Very Long File Name That Needs A Short Alias.txt",
    "fragmented.bin", "interleave.bin", "Links\target.txt", "sparse.bin") { Record $f $false "" }
Record "compressed.txt" $false "ntfs-compressed"
Record "Pictures\deleted_photo.jpg" $true ""
Record "OldStuff\letter.txt" $true ""
Record "OldStuff\inner\memo.txt" $true ""

Remove-Item (Join-Path $root "Pictures\deleted_photo.jpg")
Remove-Item (Join-Path $root "OldStuff") -Recurse

Write-VolumeCache -DriveLetter $part.DriveLetter
} finally {
    # Always detach, even on failure, so the test volume never lingers as a drive letter.
    Dismount-DiskImage -ImagePath $vhd -ErrorAction SilentlyContinue | Out-Null
}

$manifest | ConvertTo-Json -Depth 3 | Set-Content -Encoding utf8 (Join-Path $here "ntfs-small.manifest.json")
$in = [System.IO.File]::OpenRead($vhd)
$out = [System.IO.File]::Create((Join-Path $here "ntfs-small.vhd.gz"))
$gz = New-Object System.IO.Compression.GZipStream($out, [System.IO.Compression.CompressionLevel]::Optimal)
$in.CopyTo($gz); $gz.Close(); $in.Close()
Remove-Item $work -Recurse -Force
Write-Host "Wrote $(Join-Path $here 'ntfs-small.vhd.gz') ($((Get-Item (Join-Path $here 'ntfs-small.vhd.gz')).Length) bytes)"
