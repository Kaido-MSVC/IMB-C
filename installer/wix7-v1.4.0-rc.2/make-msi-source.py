from pathlib import Path
import xml.etree.ElementTree as E
import hashlib,uuid,json,re

import sys
project=Path(__file__).resolve().parents[2]
payload=Path(sys.argv[1]).resolve()
# Generated authoring and terms go to a new output folder (argv[2]); never into released folders.
out=Path(sys.argv[2]).resolve()
out.mkdir(parents=True,exist_ok=False)
expected={}
for line in (payload/'SHA256SUMS.txt').read_text(encoding='utf-8').splitlines():
    digest,name=line.split('  ',1);expected[name]=digest
actual={x.relative_to(payload).as_posix():hashlib.sha256(x.read_bytes()).hexdigest() for x in payload.rglob('*') if x.is_file()}
actual.pop('SHA256SUMS.txt')
if actual!=expected:raise SystemExit('Installer payload differs from SHA256SUMS.txt')
ns='http://wixtoolset.org/schemas/v4/wxs'
E.register_namespace('',ns)
def el(parent,tag,**attrs):return E.SubElement(parent,'{'+ns+'}'+tag,{k:str(v) for k,v in attrs.items()})
def write(p,s):p.write_text(s,encoding='utf-8',newline='\n')
# IMB-C 1.4 RC2 candidate family (separate from the 1.3.x stable UpgradeCode).
upgrade='C897A5DB-A859-46A6-BEA9-50585AE110ED'
# Simplified-terms rebuild of 1.4.0-rc.2. Earlier RC2 MSIs used CC2F9898-9DCB-440F-AC83-0DD912B72D9C
# (first build) and 4010481F-F615-48CE-8A9B-9B734351A7D7 (Runtime update).
product='46BDFC27-39F0-4717-B455-9254309F59B3'
namespace=uuid.UUID(upgrade)
def ident(prefix,path):return prefix+hashlib.sha256(path.encode()).hexdigest()[:26]
def guid(path):return str(uuid.uuid5(namespace,path.lower())).upper()

# Present the unchanged complete Runtime terms first. Append full third-party
# notices and unique license texts; none is replaced by a short installer EULA.
termsfiles=['RUNTIME-RECIPIENT-TERMS.md','LICENSE','RELEASE-NOTES-1.4.0-rc.2.md','RUNTIME-HANDOFF.md','LGPL-REPLACEMENT.md','FREETYPE-NOTICE.md','docs/v1.4-rc2-license-review.md','licenses/wix-7.0.0/WiX-7.0.0-LICENSE.TXT','licenses/wix-7.0.0/WiX-7.0.0-OSMFEULA.txt','THIRD-PARTY-LICENSES-v1.3.0.md','README.md','THIRD-PARTY-LICENSES-v1.2.0.md','THIRD-PARTY-NOTICES.md','THIRD-PARTY-RUNTIME.txt','ImageMagick/License.txt','ImageMagick/NOTICE.txt']
for p in sorted((payload/'licenses').rglob('*')):
    if p.is_file() and re.match(r'(?i)^(licen[cs]e|copying|copyright|authors|notice|patents)',p.name) and p.suffix.lower() not in ['.zip','.docx','.json','.html']:
        termsfiles.append(p.relative_to(payload).as_posix())
terms_bytes=(payload/'RUNTIME-RECIPIENT-TERMS.md').read_bytes()
terms_id=next(l.split('：',1)[1].strip().rstrip('。') for l in terms_bytes.decode('utf-8').splitlines() if l.startswith('固定版本：'))
intro=f'''IMB-C 1.4.0-rc.2 Personal Use — x64 MSI (pre-release candidate)
完整適用條款與第三方告知 / Complete terms and third-party notices

本 MSI 隨附 Microsoft Visual C++ Runtime（ImageMagick 目錄內四份未修改的 DLL）。勾選同意表示接受下列第一份文件與其附錄的 Microsoft 官方條款；其他元件依各自授權，LGPL 修改、替換與偵錯反向工程權利均保留。
完整原始條款、官方 DOCX、來源與通知材料均安裝至程式資料夾。
This MSI includes the Microsoft Visual C++ Runtime. Checking the box accepts the first document below and the official Microsoft terms it reproduces; other components keep their own licenses. The complete terms are installed with the program.
Runtime 條款固定版本 / Runtime terms version: {terms_id}
Runtime 條款 SHA-256: {hashlib.sha256(terms_bytes).hexdigest()}
\n'''
texts=[intro];seen=set();included=[]
for rel in termsfiles:
    b=(payload/rel).read_bytes();h=hashlib.sha256(b).hexdigest()
    if h in seen:continue
    seen.add(h)
    try:t=b.decode('utf-8-sig')
    except UnicodeDecodeError:t=b.decode('cp1252')
    texts.append('\n================ '+rel+' ================\n'+t+'\n')
    included.append({'path':rel,'sha256':h})
texts.append('\nEND OF COMPLETE TERMS / 完整條款結束\n')
text=''.join(texts)
write(out/'Installer-Terms.txt',text)
def rtf_escape(t):
    result=[]
    for c in t:
        n=ord(c)
        if c=='\r':continue
        if c=='\n':result.append('\\par\n')
        elif c in '{}\\':result.append('\\'+c)
        elif c=='\t':result.append('\\tab ')
        elif 32<=n<127:result.append(c)
        else:
            u=c.encode('utf-16-le')
            for i in range(0,len(u),2):
                v=int.from_bytes(u[i:i+2],'little');result.append('\\u'+str(v if v<32768 else v-65536)+'?')
    return ''.join(result)
rtf='{\\rtf1\\ansi\\ansicpg1252\\uc1\\deff0{\\fonttbl{\\f0 Segoe UI;}}\\f0\\fs18\n'+rtf_escape(text)+'}'
(out/'Installer-Terms.rtf').write_bytes(rtf.encode('ascii'))
write(out/'TERMS-INVENTORY.json',json.dumps({'runtime_terms_unchanged':True,'included':included,'characters':len(text),'rtf_bytes':len(rtf)},ensure_ascii=False,indent=2))

w=E.Element('{'+ns+'}Wix')
p=el(w,'Package',Name='IMB-C 1.4.0-rc.2 Personal Use',Manufacturer='Kaido-MSVC',Version='1.4.0',Language='1033',Codepage='65001',ProductCode=product,UpgradeCode=upgrade,Scope='perMachine',Compressed='yes',InstallerVersion='500')
el(p,'SummaryInformation',Description='IMB-C 1.4.0-rc.2 Personal Use x64 installer')
# Same-version candidates are not upgraded automatically (UPGRADE-POLICY.md): detect any
# installed build of this candidate family and require uninstalling it first.
up=el(p,'Upgrade',Id=upgrade)
el(up,'UpgradeVersion',OnlyDetect='yes',Minimum='0.0.0',IncludeMinimum='yes',Property='IMBCRC2FAMILYINSTALLED')
el(p,'MediaTemplate',EmbedCab='yes',CompressionLevel='medium')
el(p,'Launch',Condition='Installed OR VersionNT64',Message='IMB-C requires 64-bit Windows.')
el(p,'Launch',Condition='Installed OR NOT IMBCRC2FAMILYINSTALLED',Message='Another IMB-C 1.4 RC2 build is installed. Uninstall it first, then run this installer again.')
el(p,'Property',Id='ARPNOMODIFY',Value='1')
el(p,'Property',Id='ARPPRODUCTICON',Value='IMBC.exe')
el(p,'Property',Id='ARPHELPLINK',Value='') if False else None
el(p,'Property',Id='INSTALLFOLDER',Secure='yes')
el(p,'Property',Id='ACCEPTTERMS',Secure='yes')
el(p,'CustomAction',Id='RequireAcceptedTerms',Error='Read and explicitly accept the complete terms. For an authorized silent install, supply ACCEPTTERMS=1 after reading the full terms.')
seq=el(p,'InstallExecuteSequence')
el(seq,'Custom',Action='RequireAcceptedTerms',Before='InstallInitialize',Condition='NOT Installed AND ACCEPTTERMS <> "1"')
el(p,'Property',Id='DESKTOPSHORTCUT',Secure='yes')
prev=el(p,'Property',Id='PREVIOUSINSTALLDIR',Secure='yes')
search=el(prev,'RegistrySearch',Id='FindPreviousInstallDir',Root='HKLM',Key=r'Software\IMB-C RC2\Installer',Name='InstallDir',Type='directory',Bitness='always64')
el(search,'DirectorySearch',Id='ExistingPreviousInstallDir',Depth='0')
el(p,'SetProperty',Id='INSTALLFOLDER',Value='[PREVIOUSINSTALLDIR]',Before='CostInitialize',Sequence='first',Condition='PREVIOUSINSTALLDIR AND NOT INSTALLFOLDER')
el(p,'Icon',Id='IMBC.exe',SourceFile='$(var.IconFile)')
el(p,'WixVariable',Id='WixUILicenseRtf',Value='$(var.TermsFile)')
el(p,'Property',Id='WIXUI_INSTALLDIR',Value='INSTALLFOLDER')
el(p,'Property',Id='WIXUI_EXITDIALOGOPTIONALTEXT',Value='IMB-C 1.4.0-rc.2 is installed. The Microsoft Visual C++ Runtime terms you accepted are in RUNTIME-RECIPIENT-TERMS.md in the program folder.')
pf=el(p,'StandardDirectory',Id='ProgramFiles64Folder')
install=el(pf,'Directory',Id='INSTALLFOLDER',Name='IMB-C 1.4 RC2')
menu=el(p,'StandardDirectory',Id='ProgramMenuFolder');el(menu,'Directory',Id='IMBCMenu',Name='IMB-C 1.4 RC2')
el(p,'StandardDirectory',Id='DesktopFolder')
feature=el(p,'Feature',Id='MainFeature',Title='IMB-C',Level='1',AllowAbsent='no')
dirs={'.':install}
files=sorted(x for x in payload.rglob('*') if x.is_file())
for f in files:
    rel=f.relative_to(payload);current='.'
    for part in rel.parts[:-1]:
        path=part if current=='.' else current+'/'+part
        if path not in dirs:dirs[path]=el(dirs[current],'Directory',Id=ident('D',path),Name=part)
        current=path
    r=rel.as_posix();cid=ident('C',r)
    c=el(dirs[current],'Component',Id=cid,Guid=guid('file/'+r),Bitness='always64')
    ff=el(c,'File',Id='MainExe' if r=='imb-c.exe' else ident('F',r),Source='$(var.PayloadRoot)/'+r,Name=f.name,KeyPath='yes')
    if r=='imb-c.exe':
        el(ff,'Shortcut',Id='StartMenuShortcut',Directory='IMBCMenu',Name='IMB-C 1.4 RC2',WorkingDirectory='INSTALLFOLDER',Advertise='yes',Icon='IMBC.exe')
    el(feature,'ComponentRef',Id=cid)
state=el(install,'Component',Id='InstallState',Guid=guid('install-state'),Bitness='always64')
el(state,'RegistryValue',Root='HKLM',Key=r'Software\IMB-C RC2\Installer',Name='InstallDir',Type='string',Value='[INSTALLFOLDER]',KeyPath='yes')
el(state,'RemoveFolder',Id='RemoveMenuDir',Directory='IMBCMenu',On='uninstall')
el(feature,'ComponentRef',Id='InstallState')
desktop=el(p,'Component',Id='DesktopShortcutComponent',Directory='DesktopFolder',Guid=guid('desktop-shortcut'),Bitness='always64',Condition='DESKTOPSHORTCUT = "1"')
el(desktop,'Shortcut',Id='DesktopShortcut',Directory='DesktopFolder',Name='IMB-C 1.4 RC2',Target='[INSTALLFOLDER]imb-c.exe',WorkingDirectory='INSTALLFOLDER',Icon='IMBC.exe')
el(desktop,'RegistryValue',Root='HKCU',Key=r'Software\IMB-C RC2\Installer',Name='DesktopShortcut',Value='1',Type='integer',KeyPath='yes')
el(feature,'ComponentRef',Id='DesktopShortcutComponent')
ui=el(p,'UI',Id='IMBCUI')
for id,size in [('Normal','9'),('Bigger','12'),('Title','10')]:el(ui,'TextStyle',Id='WixUI_Font_'+id,FaceName='Segoe UI',Size=size,**({'Bold':'yes'} if id=='Title' else {}))
el(ui,'Property',Id='DefaultUIFont',Value='WixUI_Font_Normal')
for id in ['BrowseDlg','DiskCostDlg','ErrorDlg','FatalError','FilesInUse','MsiRMFilesInUse','PrepareDlg','ProgressDlg','ResumeDlg','UserExit','WelcomeDlg','LicenseAgreementDlg','InstallDirDlg','VerifyReadyDlg','MaintenanceWelcomeDlg','MaintenanceTypeDlg','ExitDialog']:
    el(ui,'DialogRef',Id=id)
def pub(dialog,control,event,value,condition=None,order=None,prop=False):
    a={'Dialog':dialog,'Control':control,'Property' if prop else 'Event':event,'Value':value}
    if condition:a['Condition']=condition
    if order:a['Order']=str(order)
    el(ui,'Publish',**a)
pub('ExitDialog','Finish','EndDialog','Return',order=999)
pub('WelcomeDlg','Next','NewDialog','LicenseAgreementDlg','NOT Installed')
pub('WelcomeDlg','Next','NewDialog','VerifyReadyDlg','Installed AND PATCH')
pub('LicenseAgreementDlg','Back','NewDialog','WelcomeDlg')
pub('LicenseAgreementDlg','Next','ACCEPTTERMS','1','LicenseAccepted = "1"',1,prop=True)
pub('LicenseAgreementDlg','Next','NewDialog','InstallDirDlg','LicenseAccepted = "1"',2)
pub('InstallDirDlg','Back','NewDialog','LicenseAgreementDlg')
pub('InstallDirDlg','Next','CheckTargetPath','[WIXUI_INSTALLDIR]',order=1)
pub('InstallDirDlg','Next','SetTargetPath','[WIXUI_INSTALLDIR]',order=3)
pub('InstallDirDlg','Next','NewDialog','ShortcutOptionsDlg',order=4)
pub('InstallDirDlg','ChangeFolder','_BrowseProperty','[WIXUI_INSTALLDIR]',order=1,prop=True)
pub('InstallDirDlg','ChangeFolder','SpawnDialog','BrowseDlg',order=2)
pub('BrowseDlg','OK','CheckTargetPath','[WIXUI_INSTALLDIR]',order=1)
pub('BrowseDlg','OK','SetTargetPath','[_BrowseProperty]',order=3)
pub('BrowseDlg','OK','EndDialog','Return',order=4)
pub('VerifyReadyDlg','Back','NewDialog','ShortcutOptionsDlg','NOT Installed',1)
pub('VerifyReadyDlg','Back','NewDialog','MaintenanceTypeDlg','Installed AND NOT PATCH',2)
pub('VerifyReadyDlg','Back','NewDialog','WelcomeDlg','Installed AND PATCH',2)
pub('MaintenanceWelcomeDlg','Next','NewDialog','MaintenanceTypeDlg')
pub('MaintenanceTypeDlg','RepairButton','NewDialog','VerifyReadyDlg')
pub('MaintenanceTypeDlg','RemoveButton','NewDialog','VerifyReadyDlg')
pub('MaintenanceTypeDlg','Back','NewDialog','MaintenanceWelcomeDlg')
dlg=el(ui,'Dialog',Id='ShortcutOptionsDlg',Width='370',Height='270',Title='[ProductName] Setup')
el(dlg,'Control',Id='Title',Type='Text',X='15',Y='15',Width='340',Height='25',Transparent='yes',NoPrefix='yes',Text=r'{\WixUI_Font_Bigger}Shortcuts and installation summary')
el(dlg,'Control',Id='Folder',Type='Text',X='20',Y='55',Width='330',Height='42',NoPrefix='yes',Text='Install to: [INSTALLFOLDER]')
el(dlg,'Control',Id='StartMenu',Type='Text',X='20',Y='105',Width='330',Height='20',Text='A Start menu shortcut will be created.')
el(dlg,'Control',Id='Desktop',Type='CheckBox',X='20',Y='137',Width='330',Height='20',Property='DESKTOPSHORTCUT',CheckBoxValue='1',Text='Create a &desktop shortcut (optional)')
el(dlg,'Control',Id='Line',Type='Line',X='0',Y='234',Width='370',Height='0')
back=el(dlg,'Control',Id='Back',Type='PushButton',X='180',Y='243',Width='56',Height='17',Text='&Back')
el(back,'Publish',Event='NewDialog',Value='InstallDirDlg')
next=el(dlg,'Control',Id='Next',Type='PushButton',X='236',Y='243',Width='56',Height='17',Default='yes',Text='&Next')
el(next,'Publish',Event='NewDialog',Value='VerifyReadyDlg')
cancel=el(dlg,'Control',Id='Cancel',Type='PushButton',X='304',Y='243',Width='56',Height='17',Cancel='yes',Text='Cancel')
el(cancel,'Publish',Event='SpawnDialog',Value='CancelDlg')
el(p,'UIRef',Id='WixUI_Common')
E.indent(w)
E.ElementTree(w).write(out/'Release.wxs',encoding='utf-8',xml_declaration=True)
print(json.dumps({'source':str(out/'Release.wxs'),'files':len(files),'upgrade_code':upgrade,'product_code':product,'terms_chars':len(text),'terms_files':len(included)},indent=2))
