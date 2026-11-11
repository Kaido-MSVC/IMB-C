from pathlib import Path
import xml.etree.ElementTree as E
import hashlib,uuid,json,re

import sys
project=Path(__file__).resolve().parents[2]
payload=Path(sys.argv[1]).resolve()
out=Path(__file__).resolve().parent
ns='http://wixtoolset.org/schemas/v4/wxs'
E.register_namespace('',ns)
def el(parent,tag,**attrs):return E.SubElement(parent,'{'+ns+'}'+tag,{k:str(v) for k,v in attrs.items()})
def write(p,s):p.write_text(s,encoding='utf-8',newline='\n')
upgrade='3EAD7AE1-2433-4AB2-B14B-70A661D06781'
product='61903C56-091E-486A-B0A9-581C67C010B8'
namespace=uuid.UUID(upgrade)
def ident(prefix,path):return prefix+hashlib.sha256(path.encode()).hexdigest()[:26]
def guid(path):return str(uuid.uuid5(namespace,path.lower())).upper()

# Present the unchanged complete Runtime terms first. Append full third-party
# notices and unique license texts; none is replaced by a short installer EULA.
termsfiles=['LICENSE','RELEASE-NOTES-1.3.2.md','FREETYPE-NOTICE.md','LICENSE-REVIEW-v1.3.0.md','licenses/wix-7.0.0/WiX-7.0.0-LICENSE.TXT','licenses/wix-7.0.0/WiX-7.0.0-OSMFEULA.txt','INSTALL.md','RUNTIME-HANDOFF.md','THIRD-PARTY-LICENSES-v1.3.0.md','README.md','THIRD-PARTY-LICENSES-v1.2.0.md','RUNTIME-RECIPIENT-TERMS.md','THIRD-PARTY-NOTICES.md','THIRD-PARTY-RUNTIME.txt','LGPL-REPLACEMENT.md','ImageMagick/License.txt','ImageMagick/NOTICE.txt']
for p in sorted((payload/'licenses').rglob('*')):
    if p.is_file() and re.match(r'(?i)^(licen[cs]e|copying|copyright|authors|notice|patents)',p.name) and p.suffix.lower() not in ['.zip','.docx','.json','.html']:
        termsfiles.append(p.relative_to(payload).as_posix())
intro='''IMB-C v1.3.2 Personal Use — x64 MSI (minor improvement release of 1.3.1)
完整適用條款與第三方告知 / Complete terms and third-party notices

本 MSI 供發行者本人測試與個人使用，不因安裝而授予公開上傳、任意轉交或好友再散布權。產品及第三方元件依原有適用條款；本說明不另行創設第三方授權。
請閱讀下列全文。勾選同意表示接受適用於 Microsoft Runtime 的完整接收條款；其他元件仍依各自授權，LGPL 修改、替換與偵錯反向工程權利均保留。
好友交付前仍須獨立提供固定版本完整條款並取得明確接受，安裝勾選不能取代交付前流程。完整原始條款、官方 DOCX、來源與通知材料均安裝至程式資料夾。
Runtime 條款固定版本 IMBC-RUNTIME-2026-10-27-v1.1.1-r1
Runtime 全文 SHA-256: 322f70065cac27c5ce8cd69fff0dceaac316330ea88445926a9ae60601b8bfc7
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
p=el(w,'Package',Name='IMB-C v1.3.2 Personal Use',Manufacturer='Kaido-MSVC',Version='1.3.2',Language='1033',Codepage='65001',ProductCode=product,UpgradeCode=upgrade,Scope='perMachine',Compressed='yes',InstallerVersion='500')
el(p,'SummaryInformation',Description='IMB-C v1.3.2 Personal Use x64 installer')
el(p,'MajorUpgrade',DowngradeErrorMessage='A newer version of IMB-C is already installed.',Schedule='afterInstallInitialize',AllowSameVersionUpgrades='no')
el(p,'MediaTemplate',EmbedCab='yes',CompressionLevel='medium')
el(p,'Launch',Condition='Installed OR VersionNT64',Message='IMB-C requires 64-bit Windows.')
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
search=el(prev,'RegistrySearch',Id='FindPreviousInstallDir',Root='HKLM',Key=r'Software\IMB-C\Installer',Name='InstallDir',Type='directory',Bitness='always64')
el(search,'DirectorySearch',Id='ExistingPreviousInstallDir',Depth='0')
el(p,'SetProperty',Id='INSTALLFOLDER',Value='[PREVIOUSINSTALLDIR]',Before='CostInitialize',Sequence='first',Condition='PREVIOUSINSTALLDIR AND NOT INSTALLFOLDER')
el(p,'Icon',Id='IMBC.exe',SourceFile='$(var.IconFile)')
el(p,'WixVariable',Id='WixUILicenseRtf',Value='$(var.TermsFile)')
el(p,'Property',Id='WIXUI_INSTALLDIR',Value='INSTALLFOLDER')
el(p,'Property',Id='WIXUI_EXITDIALOGOPTIONALTEXT',Value='IMB-C Personal-Use is installed. Friend delivery still requires separate prior acceptance of the complete Runtime terms.')
pf=el(p,'StandardDirectory',Id='ProgramFiles64Folder')
install=el(pf,'Directory',Id='INSTALLFOLDER',Name='IMB-C')
menu=el(p,'StandardDirectory',Id='ProgramMenuFolder');el(menu,'Directory',Id='IMBCMenu',Name='IMB-C')
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
        el(ff,'Shortcut',Id='StartMenuShortcut',Directory='IMBCMenu',Name='IMB-C',WorkingDirectory='INSTALLFOLDER',Advertise='yes',Icon='IMBC.exe')
    el(feature,'ComponentRef',Id=cid)
state=el(install,'Component',Id='InstallState',Guid=guid('install-state'),Bitness='always64')
el(state,'RegistryValue',Root='HKLM',Key=r'Software\IMB-C\Installer',Name='InstallDir',Type='string',Value='[INSTALLFOLDER]',KeyPath='yes')
el(state,'RemoveFolder',Id='RemoveMenuDir',Directory='IMBCMenu',On='uninstall')
el(feature,'ComponentRef',Id='InstallState')
desktop=el(p,'Component',Id='DesktopShortcutComponent',Directory='DesktopFolder',Guid=guid('desktop-shortcut'),Bitness='always64',Condition='DESKTOPSHORTCUT = "1"')
el(desktop,'Shortcut',Id='DesktopShortcut',Directory='DesktopFolder',Name='IMB-C',Target='[INSTALLFOLDER]imb-c.exe',WorkingDirectory='INSTALLFOLDER',Icon='IMBC.exe')
el(desktop,'RegistryValue',Root='HKCU',Key=r'Software\IMB-C\Installer',Name='DesktopShortcut',Value='1',Type='integer',KeyPath='yes')
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
el(dlg,'Control',Id='ConsentReminder',Type='Text',X='20',Y='172',Width='330',Height='40',Text='Friend delivery requires prior explicit acceptance of the complete Runtime terms. Installer acceptance does not replace that process.')
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
