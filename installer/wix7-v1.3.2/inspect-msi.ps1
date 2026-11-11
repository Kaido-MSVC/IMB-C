param([string]$Msi,[string]$Terms,[string]$Output)
$ErrorActionPreference='Stop'
$wi=New-Object -ComObject WindowsInstaller.Installer
$db=$wi.OpenDatabase((Resolve-Path -LiteralPath $Msi).Path,0)
function Query([string]$sql){
 $v=$db.OpenView($sql);[void]$v.Execute();$rows=@()
 while($rec=$v.Fetch()){$values=@();for($i=1;$i -le $rec.FieldCount();$i++){$values+= $rec.StringData($i)};$rows+=,$values}
 [void]$v.Close();return ,$rows
}
$tables=@{}
foreach($name in @('Property','File','Component','Directory','FeatureComponents','Media','Upgrade','RegLocator','Registry','Shortcut','CustomAction','ControlEvent','Dialog','InstallExecuteSequence','InstallUISequence','RemoveFile','LaunchCondition')){
 $tables[$name]=Query ('SELECT * FROM `'+$name+'`')
}
$props=@{};foreach($r in $tables.Property){$props[$r[0]]=$r[1]}
$license=Query 'SELECT `Text` FROM `Control` WHERE `Dialog_`=''LicenseAgreementDlg'' AND `Control`=''LicenseText'''
$expected=[IO.File]::ReadAllText((Resolve-Path -LiteralPath $Terms).Path)
$checks=@{
 ProductVersion=$props.ProductVersion;ProductCode=$props.ProductCode;UpgradeCode=$props.UpgradeCode
 PackageCode=$db.SummaryInformation(0).Property(9);Template=$db.SummaryInformation(0).Property(7)
 AllUsers=$props.ALLUSERS;FileCount=$tables.File.Count
 CompleteRtfInMSI=($license[0][0] -ceq $expected)
 DefaultDesktopUnchecked=(-not $props.ContainsKey('DESKTOPSHORTCUT'))
 Tables=$tables
}
if($checks.ProductVersion -ne '1.3.2'){throw 'Wrong version'}
if($checks.ProductCode -in @('{60A17426-9610-4EC4-BD3B-65E6BFC78A46}','{310C2D7F-3E57-4CBD-B8FA-B100CCEA6F2E}')){throw 'ProductCode must differ from 1.3.0 and 1.3.1'}
if($props.Manufacturer -ne 'Kaido-MSVC'){throw 'Wrong manufacturer'}
if($checks.UpgradeCode -ne '{3EAD7AE1-2433-4AB2-B14B-70A661D06781}'){throw 'Wrong upgrade identity'}
if($checks.Template -ne 'x64;1033'){throw 'Wrong platform'}
if($checks.FileCount -lt 600){throw 'Incomplete payload'}
if(-not $checks.CompleteRtfInMSI){throw 'Full terms missing'}
if($checks.AllUsers -ne '1'){throw 'Wrong installation scope'}
if(-not $checks.DefaultDesktopUnchecked){throw 'Desktop shortcut must be opt-in'}
if($tables.Shortcut.Count -ne 2){throw 'Missing shortcuts'}
foreach($a in $tables.CustomAction){if([int]$a[1] -notin @(19,51,307)){throw "Unexpected custom action $($a[0])"}}
if(-not ($tables.CustomAction | Where-Object {$_[0] -eq 'RequireAcceptedTerms' -and $_[1] -eq '19'})){throw 'Consent gate missing'}
$dialogs=@($tables.Dialog | ForEach-Object {$_[0]})
foreach($name in @('WelcomeDlg','LicenseAgreementDlg','InstallDirDlg','ShortcutOptionsDlg','VerifyReadyDlg','ProgressDlg','ExitDialog','MaintenanceTypeDlg')){if($name -notin $dialogs){throw "Missing dialog: $name"}}
foreach($table in @('InstallUISequence','InstallExecuteSequence')){
 $actions=@{};foreach($row in $tables[$table]){$actions[$row[0]]=$row}
 if([int]$actions['AppSearch'][2] -ge [int]$actions['SetINSTALLFOLDER'][2] -or [int]$actions['SetINSTALLFOLDER'][2] -ge [int]$actions['CostInitialize'][2]){throw "Remembered directory action incorrectly sequenced in $table"}
 if($actions['SetINSTALLFOLDER'][1] -notmatch 'NOT INSTALLFOLDER'){throw 'Explicit directory must win'}
}
$checks | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $Output -Encoding utf8
$checks | Select-Object ProductVersion,ProductCode,UpgradeCode,PackageCode,Template,AllUsers,FileCount,CompleteRtfInMSI,DefaultDesktopUnchecked | ConvertTo-Json
