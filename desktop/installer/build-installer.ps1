[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ExecutablePath,
    [Parameter(Mandatory = $true)][string]$OutputDir,
    [string]$Version = '',
    [string]$ProductCode = '',
    [string]$UpgradeCode = '{7D0BD59C-0911-48DC-87C5-8EAA89BFCED7}',
    [string]$ComponentCode = '{82025BFC-A311-4A2D-9099-A858FD8FF6E0}',
    [string]$WorkDir = '',
    [string]$RustcPath,
    [string]$ManifestTool,
    [switch]$MsiOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$ExecutablePath = (Resolve-Path -LiteralPath $ExecutablePath).Path
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
$null = New-Item -ItemType Directory -Path $OutputDir -Force
if ($MsiOnly -and (Test-Path -LiteralPath (Join-Path $OutputDir 'CursorCueSetup.exe'))) { throw 'Use a separate output directory for MSI-only builds to avoid leaving a stale setup wrapper.' }
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$projectParent = Split-Path -Parent $projectRoot
$workspaceRoot = if ((Split-Path -Leaf $projectParent) -eq 'outputs') { Split-Path -Parent $projectParent } else { $projectRoot }
if (!$WorkDir) { $WorkDir = Join-Path $workspaceRoot 'work\installer' }
$workRoot = [IO.Path]::GetFullPath($WorkDir)
$null = New-Item -ItemType Directory -Path $workRoot -Force
$file = Get-Item -LiteralPath $ExecutablePath
if (!$Version) { $Version = $file.VersionInfo.FileVersion }
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Version must be major.minor.patch; use a versioned CursorCue executable or supply -Version.' }
$parsedVersion = [Version]$Version
if ($parsedVersion.Major -gt 255 -or $parsedVersion.Minor -gt 255 -or $parsedVersion.Build -gt 65535) { throw 'Version exceeds Windows Installer limits.' }
if ($file.VersionInfo.FileVersion -and $file.VersionInfo.FileVersion -ne $Version) { throw 'Executable and installer versions must match.' }
$componentGuid = ([Guid]$ComponentCode).ToString('B').ToUpperInvariant()
$upgradeGuid = ([Guid]$UpgradeCode).ToString('B').ToUpperInvariant()
if (!$ProductCode) {
    $hash = [Security.Cryptography.SHA256]::Create()
    try { $digest = $hash.ComputeHash([Text.Encoding]::UTF8.GetBytes("CursorCue|$upgradeGuid|$Version")) }
    finally { $hash.Dispose() }
    $ProductCode = ([Guid]::new([byte[]]$digest[0..15])).ToString('B')
}
$productGuid = ([Guid]$ProductCode).ToString('B').ToUpperInvariant()
$iconPath = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\assets\CursorCue.ico')).Path
$registryKey = 'Software\CursorCue\Installer\' + $productGuid
$stage = Join-Path $workRoot ('CursorCue-build-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $stage
$msiPath = Join-Path $stage 'CursorCue.msi'

function Invoke-ComMethod($Object, [string]$Name, [object[]]$Arguments = @()) {
    for ($i = 0; $i -lt $Arguments.Length; $i++) { $Arguments[$i] = $Arguments[$i].PSObject.BaseObject }
    $Object.GetType().InvokeMember($Name, [Reflection.BindingFlags]::InvokeMethod, $null, $Object, $Arguments)
}

function Import-Table([string]$Name, [string[]]$Columns, [string[]]$Types, [string[]]$Keys, [object[]]$Rows) {
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add(($Columns -join "`t"))
    $lines.Add(($Types -join "`t"))
    $lines.Add((@($Name) + $Keys -join "`t"))
    foreach ($row in $Rows) { $lines.Add(($row -join "`t")) }
    [IO.File]::WriteAllText((Join-Path $stage ($Name + '.idt')), ($lines -join "`r`n") + "`r`n", [Text.Encoding]::ASCII)
    $null = Invoke-ComMethod $database 'Import' @($stage, ($Name + '.idt'))
}

$installer = $database = $summary = $null
try {
    Copy-Item -LiteralPath $ExecutablePath -Destination (Join-Path $stage 'cursorcue.exe')
    $ddf = @"
.OPTION EXPLICIT
.Set CabinetNameTemplate=payload.cab
.Set DiskDirectoryTemplate=.
.Set Cabinet=ON
.Set Compress=ON
.Set CompressionType=MSZIP
.Set MaxDiskSize=0
.Set InfFileName=payload.inf
.Set RptFileName=payload.rpt
cursorcue.exe CursorCueExe
"@
    [IO.File]::WriteAllText((Join-Path $stage 'payload.ddf'), $ddf, [Text.Encoding]::ASCII)
    $cabProcess = Start-Process -FilePath "$env:SystemRoot\System32\makecab.exe" -WorkingDirectory $stage -ArgumentList @('/F', 'payload.ddf') -Wait -PassThru -WindowStyle Hidden
    if ($cabProcess.ExitCode -ne 0 -or !(Test-Path -LiteralPath (Join-Path $stage 'payload.cab'))) { throw 'Cabinet creation failed.' }
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $database = Invoke-ComMethod $installer 'OpenDatabase' @($msiPath, 3)
    Import-Table 'Property' @('Property', 'Value') @('s72', 'l0') @('Property') @(
        ,@('ProductCode', $productGuid)
        ,@('ProductName', 'CursorCue')
        ,@('ProductVersion', $Version)
        ,@('ProductLanguage', '1033')
        ,@('Manufacturer', 'CursorCue')
        ,@('UpgradeCode', $upgradeGuid)
        ,@('INSTALLLEVEL', '1')
        ,@('ARPNOMODIFY', '1')
        ,@('ARPINSTALLLOCATION', '[INSTALLDIR]')
        ,@('SecureCustomProperties', 'INSTALLDIR;SHORTCUTDIR;OLDERPRODUCTS;NEWERPRODUCTS')
        ,@('ARPPRODUCTICON', 'CursorCueIcon')
        ,@('DefaultUIFont', 'Body')
    )
    Import-Table 'Directory' @('Directory', 'Directory_Parent', 'DefaultDir') @('s72', 'S72', 'l255') @('Directory') @(
        ,@('TARGETDIR', '', 'SourceDir')
        ,@('LocalAppDataFolder', 'TARGETDIR', '.')
        ,@('ProgramsDir', 'LocalAppDataFolder', 'Programs')
        ,@('INSTALLDIR', 'ProgramsDir', 'CursorCue')
        ,@('ProgramMenuFolder', 'TARGETDIR', '.')
        ,@('SHORTCUTDIR', 'ProgramMenuFolder', 'CursorCue')
    )
    Import-Table 'Component' @('Component', 'ComponentId', 'Directory_', 'Attributes', 'Condition', 'KeyPath') @('s72', 'S38', 's72', 'i2', 'S255', 'S72') @('Component') @(
        ,@('CursorCueComponent', $componentGuid, 'INSTALLDIR', '260', '', 'InstallMarker')
    )
    Import-Table 'Feature' @('Feature', 'Feature_Parent', 'Title', 'Description', 'Display', 'Level', 'Directory_', 'Attributes') @('s38', 'S38', 'L64', 'L255', 'I2', 'i2', 'S72', 'i2') @('Feature') @(
        ,@('MainFeature', '', 'CursorCue', 'CursorCue desktop application', '1', '1', 'INSTALLDIR', '0')
    )
    Import-Table 'FeatureComponents' @('Feature_', 'Component_') @('s38', 's72') @('Feature_', 'Component_') @(
        ,@('MainFeature', 'CursorCueComponent')
    )
    Import-Table 'File' @('File', 'Component_', 'FileName', 'FileSize', 'Version', 'Language', 'Attributes', 'Sequence') @('s72', 's72', 'l255', 'i4', 'S72', 'S20', 'I2', 'i4') @('File') @(
        ,@('CursorCueExe', 'CursorCueComponent', 'cursorcue.exe', $file.Length.ToString(), $file.VersionInfo.FileVersion, '', '512', '1')
    )
    $hashRecord = Invoke-ComMethod $installer 'FileHash' @($ExecutablePath, 0)
    Import-Table 'MsiFileHash' @('File_', 'Options', 'HashPart1', 'HashPart2', 'HashPart3', 'HashPart4') @('s72', 'i2', 'i4', 'i4', 'i4', 'i4') @('File_') @(
        ,@('CursorCueExe', '0', $hashRecord.IntegerData(1), $hashRecord.IntegerData(2), $hashRecord.IntegerData(3), $hashRecord.IntegerData(4))
    )
    $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($hashRecord)
    Import-Table 'Media' @('DiskId', 'LastSequence', 'DiskPrompt', 'Cabinet', 'VolumeLabel', 'Source') @('i2', 'i4', 'L64', 'S255', 'S32', 'S72') @('DiskId') @(
        ,@('1', '1', '', '#payload.cab', '', '')
    )
    Import-Table 'Registry' @('Registry', 'Root', 'Key', 'Name', 'Value', 'Component_') @('s72', 'i2', 'l255', 'L255', 'L0', 's72') @('Registry') @(
        ,@('InstallMarker', '1', $registryKey, 'Installed', '#1', 'CursorCueComponent')
        ,@('InstallPath', '1', $registryKey, 'InstallDir', '[INSTALLDIR]', 'CursorCueComponent')
        ,@('ShortcutPath', '1', $registryKey, 'ShortcutDir', '[SHORTCUTDIR]', 'CursorCueComponent')
    )
    Import-Table 'AppSearch' @('Property', 'Signature_') @('s72', 's72') @('Property', 'Signature_') @(
        ,@('INSTALLDIR', 'InstallDirSearch')
        ,@('INSTALLDIR', 'PreviousInstallDirSearch')
        ,@('SHORTCUTDIR', 'ShortcutDirSearch')
        ,@('SHORTCUTDIR', 'PreviousShortcutDirSearch')
        ,@('WINBUILD', 'WindowsBuildSearch')
    )
    Import-Table 'Signature' @('Signature', 'FileName', 'MinVersion', 'MaxVersion', 'MinSize', 'MaxSize', 'MinDate', 'MaxDate', 'Languages') @('s72', 'L255', 'S20', 'S20', 'I4', 'I4', 'I4', 'I4', 'S255') @('Signature') @()
    Import-Table 'RegLocator' @('Signature_', 'Root', 'Key', 'Name', 'Type') @('s72', 'i2', 's255', 'S255', 'I2') @('Signature_') @(
        ,@('InstallDirSearch', '1', $registryKey, 'InstallDir', '18')
        ,@('PreviousInstallDirSearch', '1', 'Software\CursorCue\Installer\[OLDERPRODUCTS]', 'InstallDir', '18')
        ,@('ShortcutDirSearch', '1', $registryKey, 'ShortcutDir', '18')
        ,@('PreviousShortcutDirSearch', '1', 'Software\CursorCue\Installer\[OLDERPRODUCTS]', 'ShortcutDir', '18')
        ,@('WindowsBuildSearch', '2', 'SOFTWARE\Microsoft\Windows NT\CurrentVersion', 'CurrentBuildNumber', '18')
    )
    Import-Table 'CustomAction' @('Action', 'Type', 'Source', 'Target') @('s72', 'i2', 'S72', 'S255') @('Action') @(
        ,@('SetInstallLocation', '51', 'ARPINSTALLLOCATION', '[INSTALLDIR]')
    )
    Import-Table 'Shortcut' @('Shortcut', 'Directory_', 'Name', 'Component_', 'Target', 'Arguments', 'Description', 'Hotkey', 'Icon_', 'IconIndex', 'ShowCmd', 'WkDir') @('s72', 's72', 'l128', 's72', 's255', 'S255', 'L255', 'I2', 'S72', 'I2', 'I2', 'S72') @('Shortcut') @(
        ,@('CursorCueShortcut', 'SHORTCUTDIR', 'CursorCue', 'CursorCueComponent', '[INSTALLDIR]cursorcue.exe', '', 'CursorCue', '', 'CursorCueIcon', '0', '1', 'INSTALLDIR')
    )
    Import-Table 'RemoveFile' @('FileKey', 'Component_', 'FileName', 'DirProperty', 'InstallMode') @('s72', 's72', 'L255', 's72', 'i2') @('FileKey') @(
        ,@('RemoveAppDir', 'CursorCueComponent', '', 'INSTALLDIR', '2')
        ,@('RemoveShortcutDir', 'CursorCueComponent', '', 'SHORTCUTDIR', '2')
    )
    Import-Table 'LaunchCondition' @('Condition', 'Description') @('s255', 'l255') @('Condition') @(
        ,@('NOT ALLUSERS', 'CursorCue installs for the current user only. Please remove the ALLUSERS option.')
        ,@('VersionNT64', 'CursorCue requires 64-bit Windows.')
        ,@('WINBUILD >= 19041', 'CursorCue requires Windows 11 or Windows 10 version 2004 or later (build 19041 or newer).')
        ,@('NOT NEWERPRODUCTS OR Installed', 'A newer CursorCue version is already installed.')
    )
    Import-Table 'Upgrade' @('UpgradeCode', 'VersionMin', 'VersionMax', 'Language', 'Attributes', 'Remove', 'ActionProperty') @('s38', 'S20', 'S20', 'S255', 'i4', 'S255', 's72') @('UpgradeCode', 'VersionMin', 'VersionMax', 'Language', 'Attributes') @(
        ,@($upgradeGuid, '', $Version, '', '0', '', 'OLDERPRODUCTS')
        ,@($upgradeGuid, $Version, '', '', '2', '', 'NEWERPRODUCTS')
    )
    Import-Table 'InstallExecuteSequence' @('Action', 'Condition', 'Sequence') @('s72', 'S255', 'I2') @('Action') @(
        ,@('FindRelatedProducts', '', '50')
        ,@('AppSearch', '', '100')
        ,@('LaunchConditions', '', '200')
        ,@('ValidateProductID', '', '700')
        ,@('CostInitialize', '', '800')
        ,@('FileCost', '', '900')
        ,@('CostFinalize', '', '1000')
        ,@('SetInstallLocation', '', '1001')
        ,@('InstallValidate', '', '1400')
        ,@('InstallInitialize', '', '1500')
        ,@('RemoveExistingProducts', 'OLDERPRODUCTS', '1510')
        ,@('ProcessComponents', '', '1600')
        ,@('UnpublishFeatures', '', '1800')
        ,@('RemoveShortcuts', '', '3200')
        ,@('RemoveRegistryValues', '', '3300')
        ,@('RemoveFiles', '', '3500')
        ,@('InstallFiles', '', '4000')
        ,@('CreateShortcuts', '', '4500')
        ,@('WriteRegistryValues', '', '5000')
        ,@('RegisterUser', '', '6000')
        ,@('RegisterProduct', '', '6100')
        ,@('PublishFeatures', '', '6300')
        ,@('PublishProduct', '', '6400')
        ,@('InstallFinalize', '', '6600')
    )
    Import-Table 'TextStyle' @('TextStyle', 'FaceName', 'Size', 'Color', 'StyleBits') @('s72', 'l32', 'i2', 'I4', 'I2') @('TextStyle') @(
        ,@('Body', 'Segoe UI', '9', '', '0')
        ,@('Heading', 'Segoe UI', '15', '', '1')
    )
    Import-Table 'Dialog' @('Dialog', 'HCentering', 'VCentering', 'Width', 'Height', 'Attributes', 'Title', 'Control_First', 'Control_Default', 'Control_Cancel') @('s72', 'i2', 'i2', 'i2', 'i2', 'i4', 'l128', 's50', 'S50', 'S50') @('Dialog') @(
        ,@('Welcome', '50', '50', '380', '300', '3', 'CursorCue setup', 'Next', 'Next', 'Cancel')
        ,@('Guide', '50', '50', '380', '300', '3', 'CursorCue setup', 'Install', 'Install', 'Cancel')
        ,@('Progress', '50', '50', '380', '170', '1', 'CursorCue setup', 'Status', '', '')
        ,@('Complete', '50', '50', '380', '300', '3', 'CursorCue setup', 'Finish', 'Finish', 'Finish')
        ,@('Cancelled', '50', '50', '380', '170', '3', 'CursorCue setup', 'Close', 'Close', 'Close')
        ,@('Failed', '50', '50', '380', '170', '3', 'CursorCue setup', 'Close', 'Close', 'Close')
    )
    $controls = [Collections.Generic.List[object]]::new()
    foreach ($dialog in @('Welcome', 'Guide', 'Progress', 'Complete', 'Cancelled', 'Failed')) {
        $controls.Add(@($dialog, 'Logo', 'Icon', '22', '20', '32', '32', '4194305', '', 'CursorCueLogo', '', ''))
        $controls.Add(@($dialog, 'Heading', 'Text', '66', '24', '290', '30', '1', '', '{\Heading}CursorCue for Windows', '', ''))
    }
    $controls.Add(@('Welcome', 'Intro', 'Text', '22', '76', '336', '36', '1', '', 'Keep your shared cursor still or hidden in one-on-ones, team calls, reviews and presentations.', '', ''))
    $controls.Add(@('Welcome', 'Location', 'Text', '22', '124', '336', '44', '1', '', 'Installs for your Windows account in [INSTALLDIR]. No account or browser extension required.', '', ''))
    $controls.Add(@('Welcome', 'CloseCopies', 'Text', '22', '176', '336', '40', '1', '', 'Before continuing, Quit every running CursorCue copy using its tray menu. Setup replaces older installed versions.', '', ''))
    $controls.Add(@('Welcome', 'Version', 'Text', '22', '225', '336', '24', '1', '', 'Version [ProductVersion]. Windows x64. Not code-signed.', '', ''))
    $controls.Add(@('Welcome', 'Next', 'PushButton', '220', '266', '66', '20', '3', '', 'Next', 'Cancel', ''))
    $controls.Add(@('Welcome', 'Cancel', 'PushButton', '292', '266', '66', '20', '3', '', 'Cancel', 'Next', ''))
    $guide = @(
        '1. Open CursorCue from Start. Choose your original window in the Tools menu.'
        '2. In Google Meet, Teams or Zoom, share the CursorCue Share WINDOW. Keep it open and unminimized.'
        '3. Work in the original window. Use the Tools menu to Freeze, Hide, Resume or Drop the shared cursor.'
        '4. Open Cursor size && shortcuts in the menu. Set size from 50% to 300%. Choose keys or clear a key to disable it, then Apply.'
    )
    for ($i = 0; $i -lt $guide.Length; $i++) {
        $controls.Add(@('Guide', ('Step' + $i), 'Text', '22', (72 + 44 * $i).ToString(), '336', '38', '1', '', $guide[$i], '', ''))
    }
    $controls.Add(@('Guide', 'Back', 'PushButton', '148', '266', '66', '20', '3', '', 'Back', 'Install', ''))
    $controls.Add(@('Guide', 'Install', 'PushButton', '220', '266', '66', '20', '3', '', 'Install', 'Cancel', ''))
    $controls.Add(@('Guide', 'Cancel', 'PushButton', '292', '266', '66', '20', '3', '', 'Cancel', 'Back', ''))
    $controls.Add(@('Progress', 'Status', 'Text', '22', '76', '336', '30', '1', '', 'Installing CursorCue. Please wait...', '', ''))
    $controls.Add(@('Progress', 'Bar', 'ProgressBar', '22', '120', '336', '12', '65537', '', '', '', ''))
    $controls.Add(@('Complete', 'Done', 'Text', '22', '76', '336', '36', '1', '', 'Setup completed. Open CursorCue from the Windows Start menu.', '', ''))
    $controls.Add(@('Complete', 'Share', 'Text', '22', '120', '336', '40', '1', '', 'Choose your source window, then share CursorCue Share as a window in your meeting app.', '', ''))
    $controls.Add(@('Complete', 'Customize', 'Text', '22', '168', '336', '48', '1', '', 'Cursor size && shortcuts opens your controls. Apply saves without closing the window. An unavailable shortcut stays disabled until you choose a free key.', '', ''))
    $controls.Add(@('Complete', 'Help', 'Text', '22', '225', '336', '30', '1', '', 'Find these instructions any time in How to use CursorCue in the Tools or tray menu.', '', ''))
    $controls.Add(@('Complete', 'Finish', 'PushButton', '292', '266', '66', '20', '3', '', 'Finish', 'Finish', ''))
    $controls.Add(@('Cancelled', 'Message', 'Text', '22', '76', '336', '44', '1', '', 'Setup was cancelled. You can run this installer again when ready.', '', ''))
    $controls.Add(@('Failed', 'Message', 'Text', '22', '76', '336', '44', '1', '', 'Setup could not complete. Close running CursorCue copies and try again. Windows Installer reports any installation error separately.', '', ''))
    foreach ($dialog in @('Cancelled', 'Failed')) {
        $controls.Add(@($dialog, 'Close', 'PushButton', '292', '136', '66', '20', '3', '', 'Close', 'Close', ''))
    }
    Import-Table 'Control' @('Dialog_', 'Control', 'Type', 'X', 'Y', 'Width', 'Height', 'Attributes', 'Property', 'Text', 'Control_Next', 'Help') @('s72', 's50', 's20', 'i2', 'i2', 'i2', 'i2', 'i4', 'S72', 'L0', 'S50', 'L50') @('Dialog_', 'Control') $controls.ToArray()
    Import-Table 'ControlEvent' @('Dialog_', 'Control_', 'Event', 'Argument', 'Condition', 'Ordering') @('s72', 's50', 's50', 's255', 'S255', 'I2') @('Dialog_', 'Control_', 'Event', 'Argument', 'Condition') @(
        ,@('Welcome', 'Next', 'NewDialog', 'Guide', '1', '1')
        ,@('Guide', 'Back', 'NewDialog', 'Welcome', '1', '1')
        ,@('Guide', 'Install', 'EndDialog', 'Return', '1', '1')
        ,@('Welcome', 'Cancel', 'EndDialog', 'Exit', '1', '1')
        ,@('Guide', 'Cancel', 'EndDialog', 'Exit', '1', '1')
        ,@('Complete', 'Finish', 'EndDialog', 'Return', '1', '1')
        ,@('Cancelled', 'Close', 'EndDialog', 'Return', '1', '1')
        ,@('Failed', 'Close', 'EndDialog', 'Return', '1', '1')
    )
    Import-Table 'EventMapping' @('Dialog_', 'Control_', 'Event', 'Attribute') @('s72', 's50', 's50', 's50') @('Dialog_', 'Control_', 'Event') @(,@('Progress', 'Bar', 'SetProgress', 'Progress'))
    Import-Table 'InstallUISequence' @('Action', 'Condition', 'Sequence') @('s72', 'S255', 'I2') @('Action') @(
        ,@('FindRelatedProducts', '', '50')
        ,@('AppSearch', '', '100')
        ,@('LaunchConditions', '', '200')
        ,@('CostInitialize', '', '800')
        ,@('FileCost', '', '900')
        ,@('CostFinalize', '', '1000')
        ,@('Welcome', 'NOT Installed', '1230')
        ,@('Progress', '', '1299')
        ,@('ExecuteAction', '', '1300')
        ,@('Complete', 'NOT REMOVE', '-1')
        ,@('Cancelled', '', '-2')
        ,@('Failed', '', '-3')
    )
    Import-Table 'Icon' @('Name', 'Data') @('s72', 'v0') @('Name') @()
    Import-Table 'Binary' @('Name', 'Data') @('s72', 'v0') @('Name') @()
    foreach ($entry in @(@('Icon', 'CursorCueIcon'), @('Binary', 'CursorCueLogo'))) {
        $view = Invoke-ComMethod $database 'OpenView' @("INSERT INTO ``$($entry[0])`` (``Name``, ``Data``) VALUES (?, ?)")
        $record = Invoke-ComMethod $installer 'CreateRecord' @(2)
        $null = $record.GetType().InvokeMember('StringData', [Reflection.BindingFlags]::SetProperty, $null, $record, @(1, $entry[1]))
        $null = Invoke-ComMethod $record 'SetStream' @(2, $iconPath)
        $null = Invoke-ComMethod $view 'Execute' @($record)
        $null = Invoke-ComMethod $view 'Close'
        foreach ($object in @($record, $view)) { $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($object) }
    }
    $streamView = Invoke-ComMethod $database 'OpenView' @('INSERT INTO `_Streams` (`Name`, `Data`) VALUES (?, ?)')
    $streamRecord = Invoke-ComMethod $installer 'CreateRecord' @(2)
    $null = $streamRecord.GetType().InvokeMember('StringData', [Reflection.BindingFlags]::SetProperty, $null, $streamRecord, @(1, 'payload.cab'))
    $null = Invoke-ComMethod $streamRecord 'SetStream' @(2, (Join-Path $stage 'payload.cab'))
    $null = Invoke-ComMethod $streamView 'Execute' @($streamRecord)
    $null = Invoke-ComMethod $streamView 'Close'
    $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($streamRecord)
    $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($streamView)
    $null = Invoke-ComMethod $database 'Commit'
    $summary = $database.GetType().InvokeMember('SummaryInformation', [Reflection.BindingFlags]::GetProperty, $null, $database, @(20))
    $summaryValues = @{ 1 = 1252; 2 = 'Installation Database'; 3 = 'CursorCue'; 4 = 'CursorCue'; 7 = 'x64;1033'; 9 = [Guid]::NewGuid().ToString('B').ToUpperInvariant(); 14 = 500; 15 = 10; 18 = 'CursorCue installer builder'; 19 = 2 }
    foreach ($key in $summaryValues.Keys) {
        $null = $summary.GetType().InvokeMember('Property', [Reflection.BindingFlags]::SetProperty, $null, $summary, @([int]$key, $summaryValues[$key]))
    }
    $null = Invoke-ComMethod $summary 'Persist'
    $null = Invoke-ComMethod $database 'Commit'
    $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($summary)
    $summary = $null
    $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database)
    $database = $null

    if (!$MsiOnly) {
        $setupPath = Join-Path $stage 'CursorCueSetup.exe'
        if (!$RustcPath) {
            $rustCommand = Get-Command rustc.exe -ErrorAction SilentlyContinue
            $RustcPath = if ($rustCommand) { $rustCommand.Source } else { Join-Path $workRoot '..\rust\rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustc.exe' }
        }
        if (!(Test-Path -LiteralPath $RustcPath)) { throw 'rustc is required to build the native setup wrapper. No new artifacts were published.' }
        $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
        $sdkVersion = Get-ChildItem -LiteralPath (Join-Path $sdkRoot 'Lib') -Directory | Where-Object {
            $candidateVersion = $null
            [Version]::TryParse($_.Name, [ref]$candidateVersion) -and
                (Test-Path -LiteralPath (Join-Path $sdkRoot "bin\$($_.Name)\x64\rc.exe") -PathType Leaf) -and
                ($ManifestTool -or (Test-Path -LiteralPath (Join-Path $sdkRoot "bin\$($_.Name)\x64\mt.exe") -PathType Leaf)) -and
                (Test-Path -LiteralPath (Join-Path $_.FullName 'um\x64\kernel32.lib') -PathType Leaf) -and
                (Test-Path -LiteralPath (Join-Path $_.FullName 'ucrt\x64\ucrt.lib') -PathType Leaf)
        } | Sort-Object { [Version]$_.Name } -Descending | Select-Object -First 1 -ExpandProperty Name
        if (!$sdkVersion) { throw 'A complete Windows SDK with x64 resource tools, Windows libraries, and CRT libraries is required. No new artifacts were published.' }
        if (!$ManifestTool) { $ManifestTool = Join-Path $sdkRoot "bin\$sdkVersion\x64\mt.exe" }
        if (!(Test-Path -LiteralPath $ManifestTool)) { throw 'Windows SDK mt.exe is required to embed the asInvoker manifest. No new artifacts were published.' }
        $vswherePath = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        if (!(Test-Path -LiteralPath $vswherePath)) { throw 'Visual Studio Installer is required to locate the C++ build tools.' }
        $vsInstallation = & $vswherePath -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($LASTEXITCODE -ne 0 -or !$vsInstallation) { throw 'Visual Studio C++ x64 build tools were not found.' }
        $vcRoot = Join-Path $vsInstallation 'VC\Tools\MSVC'
        $vcVersionDir = (Get-ChildItem -LiteralPath $vcRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1).FullName
        $previousPath = $env:PATH
        $previousLib = $env:LIB
        $previousPayload = $env:CURSORCUE_MSI_PATH
        try {
            $env:PATH = (Join-Path $vcVersionDir 'bin\Hostx64\x64') + ';' + $previousPath
            $env:LIB = (@((Join-Path $vcVersionDir 'lib\x64'), (Join-Path $sdkRoot "Lib\$sdkVersion\um\x64"), (Join-Path $sdkRoot "Lib\$sdkVersion\ucrt\x64")) -join ';')
            $env:CURSORCUE_MSI_PATH = $msiPath
            $setupRc = Join-Path $stage 'setup.rc'
            $setupRes = Join-Path $stage 'setup.res'
            $versionNumbers = $Version.Replace('.', ',') + ',0'
            $resourceText = @"
1 ICON "$($iconPath.Replace('\', '/'))"
1 VERSIONINFO
 FILEVERSION $versionNumbers
 PRODUCTVERSION $versionNumbers
 FILEOS 0x40004
 FILETYPE 1
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "FileDescription", "CursorCue setup"
   VALUE "ProductName", "CursorCue"
   VALUE "FileVersion", "$Version"
   VALUE "ProductVersion", "$Version"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x409, 1200
 END
END
"@
            [IO.File]::WriteAllText($setupRc, $resourceText, [Text.Encoding]::UTF8)
            & (Join-Path $sdkRoot "bin\$sdkVersion\x64\rc.exe") '/nologo' '/c65001' '/fo' $setupRes $setupRc
            if ($LASTEXITCODE -ne 0) { throw 'Setup icon resource compilation failed.' }
            & $RustcPath (Join-Path $PSScriptRoot 'bootstrapper.rs') '--edition=2021' '--target' 'x86_64-pc-windows-msvc' "--remap-path-prefix=$env:USERPROFILE=/build-user" "--remap-path-prefix=$projectRoot=/cursorcue" '-C' 'opt-level=z' '-C' 'panic=abort' '-C' 'strip=symbols' '-C' 'debuginfo=0' '-C' 'target-feature=+crt-static' '-C' "link-arg=$setupRes" '-o' $setupPath
            if ($LASTEXITCODE -ne 0) { throw 'Native setup wrapper compilation failed. No new artifacts were published.' }
            $setupManifest = Join-Path $stage 'setup.manifest'
            [IO.File]::WriteAllText($setupManifest, [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'setup.manifest')).Replace('version="0.0.0.0"', ('version="' + $Version + '.0"')), [Text.Encoding]::UTF8)
            & $ManifestTool '-nologo' '-manifest' $setupManifest ("-outputresource:$setupPath;#1")
            if ($LASTEXITCODE -ne 0) { throw 'Native setup manifest embedding failed. No new artifacts were published.' }
        }
        finally {
            $env:PATH = $previousPath
            $env:LIB = $previousLib
            $env:CURSORCUE_MSI_PATH = $previousPayload
        }
    }
    $publishedNames = @('CursorCue.msi')
    if (!$MsiOnly) { $publishedNames += 'CursorCueSetup.exe' }
    foreach ($name in $publishedNames) {
        $destination = Join-Path $OutputDir $name
        if (Test-Path -LiteralPath $destination) { Copy-Item -LiteralPath $destination -Destination (Join-Path $stage ('previous-' + $name)) }
    }
    try {
        foreach ($name in $publishedNames) { Copy-Item -LiteralPath (Join-Path $stage $name) -Destination (Join-Path $OutputDir $name) -Force }
    } catch {
        foreach ($name in $publishedNames) {
            $previous = Join-Path $stage ('previous-' + $name)
            $destination = Join-Path $OutputDir $name
            if (Test-Path -LiteralPath $previous) {
                if (!(Test-Path -LiteralPath $destination) -or (Get-FileHash $previous).Hash -ne (Get-FileHash $destination).Hash) { Copy-Item -LiteralPath $previous -Destination $destination -Force }
            } elseif (Test-Path -LiteralPath $destination) { Remove-Item -LiteralPath $destination -Force }
        }
        throw
    }
    Get-Item -LiteralPath (Join-Path $OutputDir 'CursorCue.msi')
    if (!$MsiOnly) { Get-Item -LiteralPath (Join-Path $OutputDir 'CursorCueSetup.exe') }
}
finally {
    foreach ($comObject in @($summary, $database, $installer)) {
        if ($null -ne $comObject) { $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($comObject) }
    }
    $stageFullPath = [IO.Path]::GetFullPath($stage)
    $cleanupRoot = $workRoot.TrimEnd('\') + '\'
    if ($stageFullPath.StartsWith($cleanupRoot, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($stageFullPath).StartsWith('CursorCue-build-')) {
        Remove-Item -LiteralPath $stageFullPath -Recurse -Force
    }
}
