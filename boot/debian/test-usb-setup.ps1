# End-to-end test of the USB installer WITHOUT touching a real stick:
#   1. creates a 3 GB virtual disk (fixed VHD) and attaches it,
#   2. finds its disk number via the VHD path (never guessed),
#   3. lets RAMForge-USB.exe set it up exactly like a stick (diskpart, FAT32, copy),
#   4. detaches it and copies it to boot\debian\out\test\stick.vhd,
#   5. boots that disk in QEMU with Secure Boot on (build.ps1 -Stage test -Tests vhd-sb).
# Needs an elevated PowerShell (diskpart, attach vdisk).
param(
    [string]$Version = "0.3.0",
    [switch]$SkipBoot
)
$ErrorActionPreference = "Stop"
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$out = Join-Path $PSScriptRoot "out"
$iso = Join-Path $out "RAMForge-Boot-$Version.iso"
$exe = Join-Path $root "target\release\RAMForge-USB.exe"
$vhd = Join-Path $env:TEMP "ramforge-usb-test.vhd"
if (-not (Test-Path $iso)) { throw "ISO fehlt: $iso" }
if (-not (Test-Path $exe)) { throw "Fehlt: $exe (cargo build --release)" }

function Invoke-Diskpart([string]$script) {
    $f = Join-Path $env:TEMP "ramforge-test-diskpart.txt"
    Set-Content -Path $f -Value $script -Encoding ASCII
    $o = diskpart /s $f | Out-String
    Remove-Item $f
    return $o
}

if (Test-Path $vhd) {
    Invoke-Diskpart "select vdisk file=`"$vhd`"`ndetach vdisk noerr`n" | Out-Null
    Remove-Item $vhd -Force
}
"== VHD anlegen und einbinden"
Invoke-Diskpart "create vdisk file=`"$vhd`" maximum=3072 type=fixed`nselect vdisk file=`"$vhd`"`nattach vdisk`n" | Out-Null
$disk = Get-DiskImage -ImagePath $vhd | Get-Disk
"Virtuelle Platte: Nummer $($disk.Number), Bus $($disk.BusType), $([math]::Round($disk.Size/1GB,2)) GB"
if ($disk.BusType -notmatch "Virtual") { throw "Sicherheitsabbruch: Datenträger $($disk.Number) ist nicht virtuell ($($disk.BusType))" }
Initialize-Disk -Number $disk.Number -PartitionStyle MBR -ErrorAction SilentlyContinue

try {
    "== RAMForge-USB.exe richtet die virtuelle Platte ein"
    $p = Start-Process -FilePath $exe -ArgumentList @("--setup-disk", $disk.Number, "--confirm-erase", $disk.Number,
        "--allow-virtual-disk", "--iso", $iso) -Wait -PassThru -NoNewWindow
    if ($p.ExitCode -ne 0) { throw "RAMForge-USB.exe Exit $($p.ExitCode)" }
    $letter = (Get-Partition -DiskNumber $disk.Number | Where-Object DriveLetter | Select-Object -First 1).DriveLetter
    "== Inhalt von ${letter}:"
    Get-ChildItem "${letter}:\" -Force | Format-Table Mode, Name, Length | Out-String
    Get-Content "${letter}:\RAMForge\stick.json"
    (Get-Volume -DriveLetter $letter | Select-Object FileSystem, FileSystemLabel, Size | Format-List | Out-String)
}
finally {
    Invoke-Diskpart "select vdisk file=`"$vhd`"`ndetach vdisk`n" | Out-Null
}
New-Item -ItemType Directory -Force (Join-Path $out "test") | Out-Null
Copy-Item $vhd (Join-Path $out "test\stick.vhd") -Force
Remove-Item $vhd -Force
"stick.vhd bereit: $(Join-Path $out 'test\stick.vhd')"
if (-not $SkipBoot) { & (Join-Path $PSScriptRoot "build.ps1") -Stage test -Tests vhd-sb -Version $Version }
