[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$MsiPath,
    [Parameter(Mandatory = $true)][string]$ExecutablePath,
    [string]$SetupPath,
    [string]$TestDir = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$MsiPath = (Resolve-Path -LiteralPath $MsiPath).Path
$ExecutablePath = (Resolve-Path -LiteralPath $ExecutablePath).Path
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$projectParent = Split-Path -Parent $projectRoot
$taskRoot = if ((Split-Path -Leaf $projectParent) -eq 'outputs') { Split-Path -Parent $projectParent } else { $projectRoot }
if (!$TestDir) { $TestDir = Join-Path $taskRoot 'work\installer-test' }
$TestDir = [IO.Path]::GetFullPath($TestDir)
if (!$TestDir.StartsWith(($taskRoot.TrimEnd('\') + '\work\'), [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Installer test directory must stay within this task work directory.'
}
$null = New-Item -ItemType Directory -Path $TestDir -Force
$installDir = Join-Path $TestDir 'app'
$shortcutDir = Join-Path $TestDir 'shortcuts'
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.OpenDatabase($MsiPath, 0)
$view = $database.OpenView('SELECT `Value` FROM `Property` WHERE `Property` = ''ProductCode''')
$view.Execute()
$record = $view.Fetch()
$productCode = $record.StringData(1)
$view.Close()
$upgradeView = $database.OpenView('SELECT `Value` FROM `Property` WHERE `Property` = ''UpgradeCode''')
$upgradeView.Execute()
$upgradeRecord = $upgradeView.Fetch()
$upgradeCode = $upgradeRecord.StringData(1)
$upgradeView.Close()
if ($upgradeCode -eq '{7D0BD59C-0911-48DC-87C5-8EAA89BFCED7}') {
    throw 'Lifecycle tests require an isolated test UpgradeCode and ComponentCode; never test against the real installed CursorCue family.'
}
$related = $installer.GetType().InvokeMember('RelatedProducts', [Reflection.BindingFlags]::GetProperty, $null, $installer, @($upgradeCode))
if ($related.GetType().InvokeMember('Count', [Reflection.BindingFlags]::GetProperty, $null, $related, @()) -gt 0) { throw 'Test refuses to upgrade an already registered related product.' }
foreach ($object in @($upgradeRecord, $upgradeView, $related)) { $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($object) }
foreach ($object in @($record, $view, $database)) { $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($object) }
$buildSession = $installer.OpenPackage($MsiPath, 1)
$buildDatabase = $buildSession.GetType().InvokeMember('Database', [Reflection.BindingFlags]::GetProperty, $null, $buildSession, @())
$buildView = $buildDatabase.OpenView('SELECT `Condition` FROM `LaunchCondition` WHERE `Condition` = ''WINBUILD >= 19041''')
$buildView.Execute()
$buildRecord = $buildView.Fetch()
if ($null -eq $buildRecord) { throw 'The MSI is missing its Windows build requirement.' }
$buildCondition = $buildRecord.StringData(1)
$buildView.Close()
foreach ($case in @(
    @{ Build = '7601'; Expected = 0 },
    @{ Build = '9600'; Expected = 0 },
    @{ Build = '10240'; Expected = 0 },
    @{ Build = '18363'; Expected = 0 },
    @{ Build = '19040'; Expected = 0 },
    @{ Build = '19041'; Expected = 1 },
    @{ Build = '22000'; Expected = 1 },
    @{ Build = '26100'; Expected = 1 },
    @{ Build = '9999'; Expected = 0 },
    @{ Build = '100000'; Expected = 1 },
    @{ Build = ''; Expected = 0 },
    @{ Build = 'invalid'; Expected = 0 }
)) {
    $null = $buildSession.GetType().InvokeMember('Property', [Reflection.BindingFlags]::SetProperty, $null, $buildSession, @('WINBUILD', [string]$case.Build))
    $conditionResult = $buildSession.GetType().InvokeMember('EvaluateCondition', [Reflection.BindingFlags]::InvokeMethod, $null, $buildSession, @([string]$buildCondition))
    if ($conditionResult -ne $case.Expected) { throw "Incorrect Windows build requirement result for '$($case.Build)'." }
}
foreach ($object in @($buildRecord, $buildView, $buildDatabase, $buildSession)) { $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($object) }
Write-Output 'PASS: Native MSI condition rejects older or invalid Windows builds and compares build numbers numerically.'
$state = $installer.ProductState($productCode)
$null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer)
if ($state -ne -1) { throw "Test refuses to modify an already registered product $productCode." }

try {
    if ($SetupPath) {
        $SetupPath = (Resolve-Path -LiteralPath $SetupPath).Path
        $install = Start-Process -FilePath $SetupPath -ArgumentList @('/quiet', ('INSTALLDIR="' + $installDir + '"'), ('SHORTCUTDIR="' + $shortcutDir + '"')) -Wait -PassThru -WindowStyle Hidden
    }
    else {
        $install = Start-Process -FilePath "$env:SystemRoot\System32\msiexec.exe" -ArgumentList @('/i', ('"' + $MsiPath + '"'), '/qn', '/norestart', ('INSTALLDIR="' + $installDir + '"'), ('SHORTCUTDIR="' + $shortcutDir + '"'), '/L*v', ('"' + (Join-Path $TestDir 'install.log') + '"')) -Wait -PassThru -WindowStyle Hidden
    }
    if ($install.ExitCode -notin @(0, 3010)) { throw "MSI installation failed with $($install.ExitCode). See $TestDir\install.log." }
    $installedExe = Join-Path $installDir 'cursorcue.exe'
    if ((Get-FileHash -LiteralPath $installedExe).Hash -ne (Get-FileHash -LiteralPath $ExecutablePath).Hash) { throw 'Installed executable hash does not match the input.' }
    $shortcutPath = Join-Path $shortcutDir 'CursorCue.lnk'
    if (!(Test-Path -LiteralPath $shortcutPath)) { throw 'The Start menu shortcut was not created.' }
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($shortcutPath)
    if ($shortcut.TargetPath -ne $installedExe) { throw 'Shortcut does not point to the installed executable.' }
    $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
    $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    Write-Output 'PASS: Silent per-user install, executable hash, and shortcut target.'
}
finally {
    $uninstall = Start-Process -FilePath "$env:SystemRoot\System32\msiexec.exe" -ArgumentList @('/x', $productCode, '/qn', '/norestart', ('INSTALLDIR="' + $installDir + '"'), ('SHORTCUTDIR="' + $shortcutDir + '"'), '/L*v', ('"' + (Join-Path $TestDir 'uninstall.log') + '"')) -Wait -PassThru -WindowStyle Hidden
    if ($uninstall.ExitCode -notin @(0, 3010, 1605)) { throw "MSI uninstall failed with $($uninstall.ExitCode). See $TestDir\uninstall.log." }
}
if ((Test-Path -LiteralPath (Join-Path $installDir 'cursorcue.exe')) -or (Test-Path -LiteralPath (Join-Path $shortcutDir 'CursorCue.lnk'))) {
    throw 'Uninstall left application or shortcut files behind.'
}
$installer = New-Object -ComObject WindowsInstaller.Installer
if ($installer.ProductState($productCode) -ne -1) { throw 'Uninstall left the product registered.' }
$null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer)
if (Test-Path -LiteralPath ('HKCU:\Software\CursorCue\Installer\' + $productCode)) { throw 'Uninstall left its registry marker behind.' }
Write-Output 'PASS: Silent uninstall removed executable, shortcut, registry marker, and product registration.'
if ($SetupPath) { Write-Output 'PASS: Native setup wrapper installed the matching application.' }
