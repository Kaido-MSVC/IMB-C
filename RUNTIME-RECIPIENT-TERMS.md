# IMB-C 1.4.0-rc.2：Microsoft Visual C++ Runtime 接收條款

固定版本：IMBC-RUNTIME-1.4.0-rc.2-r3。

本文件適用於隨 IMB-C 1.4.0-rc.2 Portable ZIP 與 MSI 提供的 Microsoft Visual C++ Runtime 檔案。下方附錄為 Microsoft 官方英文條款全文，中文與英文說明只描述適用範圍，不取代官方全文；說明與官方全文不一致時，以官方全文為準。

1. 適用檔案：`ImageMagick` 目錄內的 `MSVCP140.dll`、`vcruntime140.dll`、`vcruntime140_1.dll`（Microsoft C/C++ Runtime）與 `vcomp140.dll`（Microsoft C/C++ OpenMP Runtime），共四份，檔案版本 14.52.37029.0，均為 Microsoft 簽署、未經修改的原檔，所有權屬於 Microsoft 及其供應商。IMB-C 的 `imb-c.exe` 與 `imb-c-cli.exe` 以靜態 CRT 建置，不載入這些 DLL；它們只供隨包的 ImageMagick 引擎及其模組使用。
2. 使用這些檔案，即表示您同意下方附錄的 Microsoft 官方條款（Microsoft Visual C++ V14 Redistributable and Runtime，Last Updated: October 1, 2025，EULA ID: Cpp_v14_ENU.1033），包括其中的技術限制、反向工程限制及其例外、貿易法令、擔保排除與責任限制。
3. 依 Microsoft 條款，您不得單獨抽取、修改、再散布、出租或單獨提供這些 DLL，也不得移除、遮蔽或修改 Microsoft 的告知；請勿暗示 Microsoft 製作或背書 IMB-C。若要讓他人使用 IMB-C，請讓對方從原發行來源取得完整封包。若您本身持有有效的 Visual Studio 授權，並依其 Distributable Code 條款自行散布，須遵守您自己的授權與 REDIST 清單，不能只憑本文件認定可以再散布。
4. 本文件隨 Portable ZIP 與 MSI 一併提供。MSI 安裝時須在授權對話方塊勾選同意；使用 Portable ZIP 時，使用這些檔案即表示同意。若不同意，請勿安裝或使用 IMB-C。IMB-C 不另外記錄同意狀態，也不會因此限制轉檔。
5. 上述限制只適用於 Microsoft 元件。ImageMagick、GLib、FriBidi、liblqr、libheif 及其他開源元件依各自授權提供。LGPL 函式庫的修改、替換、為偵錯修改而進行的反向工程，以及依 LGPL 取得對應來源的權利均保留；本文件不限制這些權利，也不使 Microsoft DLL 改採開源授權。
6. 發行者依其 Visual Studio Community 2026 授權的 Distributable Code 條款散布這些檔案，並自行承擔該條款對 Microsoft 的補償與防禦義務，使用者不承擔此義務。不可排除的法定權利，以及官方條款中的地區差異條款，仍然適用。

## English summary

This document covers the four unmodified, Microsoft-signed Visual C++ Runtime files (version 14.52.37029.0) in the `ImageMagick` folder of the IMB-C 1.4.0-rc.2 Portable ZIP and MSI. `imb-c.exe` and `imb-c-cli.exe` are built with the static CRT and do not load them; only the bundled ImageMagick engine does. By using these files you agree to the official Microsoft license terms reproduced below, which prevail over this summary. Do not extract, modify, redistribute or separately provide these files, and do not remove Microsoft notices. The MSI asks you to accept these terms in its license dialog; for the Portable ZIP, using the files means you accept them. If you do not agree, do not install or use IMB-C. IMB-C does not record acceptance and does not restrict conversion. Open-source components, including the LGPL libraries, remain under their own licenses; the rights to modify and replace LGPL libraries, to reverse-engineer for debugging such modifications, and to obtain the corresponding source are not restricted by this document.

官方來源 / Official source: https://visualstudio.microsoft.com/license-terms/vs2026-ga-visualcpp-v14-redist-runtime/
官方 DOCX 原件 / Official DOCX: `licenses/microsoft/Runtime-2026.docx`，SHA-256 `08651651a7602fc7c0e2763de0fde1ff9f868df2780597cd1775ee9d6441c783`。
下文由 DOCX 擷取段落文字，清單編號與版面以原件為準 / Text extracted from the DOCX; see the original for list numbering and layout.

---

MICROSOFT SOFTWARE LICENSE TERMS
MICROSOFT VISUAL C++ V14 REDISTRIBUTABLE and RUNTIME 
Last Updated: October 1, 2025
These license terms are an agreement between Microsoft Corporation (or based on where you live, one of its affiliates) and you. They apply to the software named above. The terms also apply to any Microsoft services or updates for the software, except to the extent those have different terms.
Unless otherwise stated, these license terms remain in effect for this version of the software and any updates or revisions released. Microsoft may update or modify these license terms at any time. It is your responsibility to regularly review the license terms and any related announcements. The most current version of the license terms will always be available at https://aka.ms/VisualStudioLicenses. By continuing to use the software after changes are posted, you agree to the updated terms.
BY USING THE SOFTWARE, YOU ACCEPT THESE TERMS. IF YOU DO NOT ACCEPT THEM, DO NOT USE THE SOFTWARE.
IF YOU COMPLY WITH THESE LICENSE TERMS, YOU HAVE THE RIGHTS BELOW.
INSTALLATION AND USE RIGHTS. 
You may install and use any number of copies of the software.
FEEDBACK. If you give feedback, suggestions, or recommendations (collectively, “Feedback”) about the software to Microsoft, you give to Microsoft, without charge, the right to use, share, and commercialize your Feedback in any way and for any purpose. You will not give Feedback that is a third party’s confidential information or subject to a license that requires Microsoft to license its software or documentation to third parties because we include your Feedback in them. These rights survive this agreement.
TERMS FOR SPECIFIC COMPONENTS.
Third Party or Open Source Components. The software may include third party or open source components with separate legal notices or governed by other agreements, as may be described in the notices file(s) accompanying the software. 
SCOPE OF LICENSE. The software is licensed, not sold. These license terms only give you some rights to use the software. Microsoft reserves all other rights. Unless applicable law gives you more rights despite this limitation, you may use the software only as expressly permitted in these license terms. In doing so, you must comply with any technical limitations in the software that only allow you to use it in certain ways. For example, if Microsoft technically limits or disables extensibility for the software, you may not extend the software by, among other things, loading or injecting into the software any non-Microsoft add-ins, macros, or packages; modifying the software registry settings; or adding features or functionality equivalent to that found in Microsoft products and services. You may not
work around any technical limitations in the software;
reverse engineer, decompile or disassemble the software, or otherwise attempt to derive the source code for the software, except and only to the extent required by third party licensing terms governing use of certain open source components that may be included in the software;
remove, minimize, block or modify any notices of Microsoft or its suppliers in the software; 
use the software in any way that is against the law; 
share, publish, rent, lease, or otherwise distribute the software or any of its code; or 
provide the software as a stand-alone offering or combined with any of your applications for others to use, or transfer the software or these license terms to any third party.
COMPLIANCE WITH TRADE LAWS. Microsoft products, software, technology, and services (“Items”) may be subject to U.S. and other countries’ export jurisdictions. Each party will comply with all laws and regulations applicable to the import or export of the Items, including, without limitation, trade laws of the U.S., EU, and UK, such as the U.S. Export Administration Regulations, sanctions regulations administered by the U.S. Office of Foreign Assets Control, the EU Dual Use Regulation 2021/821, and/or other end-user, end use, and destination restrictions (“Trade Laws”) as well as the global legal compliance standards detailed in the Microsoft Standards of Business Conduct. You will not, and will ensure your Affiliates will not, take any action that causes Microsoft to violate applicable Trade Laws. Microsoft may suspend or terminate this agreement immediately without notice to the extent that Microsoft reasonably believes that performance would cause it to violate Trade Laws or put it at risk of becoming subject to sanctions and penalties under such laws. You remain responsible for its and for your Affiliates’ compliance with this section. For additional information, see www.microsoft.com/exporting.
You must ensure that you, any of your directors or officers, or to the best of your knowledge, your employees or agents, Affiliates, or Affiliates’ directors, officers, employees, or agents are not sanctioned persons designated under applicable Trade Laws.
You shall provide Microsoft with prompt written notice if you or any of your Affiliates receive notice from any government agency of any investigation, inquiry, or proceeding for any actual or alleged noncompliance of or with any Trade Laws related to activities subject to this Agreement.
You shall comply with global use restrictions in Microsoft’s product or service terms relevant to this agreement, as updated from time to time.
You shall not make available Microsoft Items to any entity or individual designated on any export or sanctions restriction lists under Trade Laws, in each case, without specific government authorization required by Trade Laws.
For the purposes of this Section 8, “Affiliate” means any legal entity that directly or indirectly owns, is owned by, or is commonly owned with the applicable party. “Own” means having more than 50% ownership or the right to direct the management of the entity.
SUPPORT. Because the software is “as is,” we may not provide support services for it.
ENTIRE AGREEMENT. These license terms, and the terms for supplements, updates, Internet-based services and support services, are the entire agreement for the software and support services.
APPLICABLE LAW. If you acquired the software in the United States, Washington state law applies to interpretation of and claims for breach of these license terms, and the laws of the state where you live apply to all other claims. If you acquired the software in any other country, its laws apply.
CONSUMER RIGHTS; REGIONAL VARIATIONS. These license terms describe certain legal rights. You may have other rights, including consumer rights, under the laws of your state or country. Separate and apart from your relationship with Microsoft, you may also have rights with respect to the party from which you acquired the software. These license terms do not change those other rights if the laws of your state or country do not permit it to do so. For example, if you acquired the software in one of the below regions, or if mandatory country law applies, then the following provisions apply to you:
Australia. You have statutory guarantees under the Australian Consumer Law and nothing in these license terms is intended to affect those rights.
Canada. You may stop receiving updates on your device by turning off Internet access. If and when you re-connect to the Internet, the software will resume checking for and installing updates. The product documentation, if any, may also specify how to turn off updates for your specific device or software.
Germany and Austria.
Warranty. The properly licensed software will perform substantially as described in any Microsoft materials that accompany the software. However, Microsoft gives no contractual guarantee in relation to the licensed software.
Limitation of Liability. In case of intentional conduct, gross negligence, claims based on the Product Liability Act, as well as, in case of death or personal or physical injury, Microsoft is liable according to the statutory law.
Subject to the foregoing clause (ii), Microsoft will only be liable for slight negligence if Microsoft is in breach of such material contractual obligations, the fulfillment of which facilitate the due performance of these license terms, the breach of which would endanger the purpose of these license terms and the compliance with which a party may constantly trust in (so-called "cardinal obligations"). In other cases of slight negligence, Microsoft will not be liable for slight negligence.
DISCLAIMER OF WARRANTY. The software is licensed “as-is.” You bear the risk of using it. Microsoft gives no express warranties, guarantees or conditions. To the extent permitted under your local laws, Microsoft excludes the implied warranties of merchantability, fitness for a particular purpose and non-infringement.
LIMITATION ON DAMAGES. You can recover from Microsoft and its suppliers only direct damages up to U.S. $5.00. You cannot recover any other damages, including consequential, lost profits, special, indirect or incidental damages.
This limitation applies to (a) anything related to the software, services, content (including code) on third party Internet sites, or third party applications; and (b) claims for breach of contract, breach of warranty, guarantee or condition, strict liability, negligence, or other tort to the extent permitted by applicable law.
It also applies even if Microsoft knew or should have known about the possibility of the damages. The above limitation or exclusion may not apply to you because your country may not allow the exclusion or limitation of incidental, consequential or other damages.

Please note: As this software is distributed in Canada, some of the clauses in this agreement are provided below in French. 
Remarque: Ce logiciel étant distribué au Canada, certaines des clauses dans ce contrat sont fournies ci-dessous en français. 
EXONÉRATION DE GARANTIE. Le logiciel visé par une licence est offert « tel quel ». Toute utilisation de ce logiciel est à votre seule risque et péril. Microsoft n’accorde aucune autre garantie expresse. Vous pouvez bénéficier de droits additionnels en vertu du droit local sur la protection des consommateurs, que ce contrat ne peut modifier. La ou elles sont permises par le droit locale, les garanties implicites de qualité marchande, d’adéquation à un usage particulier et d’absence de contrefaçon sont exclues. 
LIMITATION DES DOMMAGES-INTÉRÊTS ET EXCLUSION DE RESPONSABILITÉ POUR LES DOMMAGES. Vous pouvez obtenir de Microsoft et de ses fournisseurs une indemnisation en cas de dommages directs uniquement à hauteur de 5,00 $ US. Vous ne pouvez prétendre à aucune indemnisation pour les autres dommages, y compris les dommages spéciaux, indirects ou accessoires et pertes de bénéfices. 
Cette limitation concerne: 
tout ce qui est relié au logiciel, aux services ou au contenu (y compris le code) figurant sur des sites Internet tiers ou dans des programmes tiers; et 
les réclamations au titre de violation de contrat ou de garantie, ou au titre de responsabilité stricte, de négligence ou d’une autre faute dans la limite autorisée par la loi en vigueur. 
Elle s’applique également, même si Microsoft connaissait ou devrait connaître l’éventualité d’un tel dommage. Si votre pays n’autorise pas l’exclusion ou la limitation de responsabilité pour les dommages indirects, accessoires ou de quelque nature que ce soit, il se peut que la limitation ou l’exclusion ci-dessus ne s’appliquera pas à votre égard. 
EFFET JURIDIQUE. Le présent contrat décrit certains droits juridiques. Vous pourriez avoir d’autres droits prévus par les lois de votre pays. Le présent contrat ne modifie pas les droits que vous confèrent les lois de votre pays si celles-ci ne le permettent pas.

EULA ID: Cpp_v14_ENU.1033
