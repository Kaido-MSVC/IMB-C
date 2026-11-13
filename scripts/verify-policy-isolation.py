"""Policy / config isolation test (2026-11-13): load the actual CORE_RL_MagickCore_.dll and verify that every external source is ignored.

Does not modify the system registry: RegLoadAppKey loads a private application hive (a standalone file, invisible in the system registry namespace,
no administrator needed), then RegOverridePredefKey redirects this process's HKLM / HKCU to that hive. The hive contains
ImageMagick's real lookup path SOFTWARE\\ImageMagick\\7.1.2\\Q:16 (ConfigurePath / CoderModulesPath / FilterModulesPath / LibPath).

Hostile sources (all pointing at the hostile directory H): the registry values above, MAGICK_CONFIGURE_PATH, MAGICK_HOME, MAGICK_CODER_MODULE_PATH,
MAGICK_FILTER_MODULE_PATH, HOME / USERPROFILE / APPDATA / LOCALAPPDATA / XDG_CONFIG_HOME, the current directory; H holds a permissive policy.xml
containing a stealth rule, a delegates.xml containing an external command, and a substitute coder module (IM_MOD_RL_svg_.dll; loading it is detectable via its module path).

Usage (each run is a separate process so the DLL loads fresh):
  python -X utf8 -I scripts/verify-policy-isolation.py <runtime ImageMagick dir> <work dir> [--expect-permissive]
--expect-permissive is the positive control: the runtime's own policy.xml is replaced with the permissive version, so the harness must be able to detect the grant.
"""
import ctypes
import json
import os
import shutil
import sys
from ctypes import wintypes
from pathlib import Path

runtime = Path(sys.argv[1]).resolve()
work = Path(sys.argv[2]).resolve()
expect_permissive = '--expect-permissive' in sys.argv
hostile = work / 'hostile'
for d in [hostile, hostile / 'coders', hostile / 'filters', hostile / '.config/ImageMagick', hostile / 'ImageMagick']:
    d.mkdir(parents=True, exist_ok=True)
PERMISSIVE = '''<?xml version="1.0" encoding="UTF-8"?>
<policymap>
  <policy domain="coder" rights="read|write" pattern="RGB" stealth="true"/>
  <policy domain="coder" rights="read|write" pattern="SVG" stealth="true"/>
  <policy domain="module" rights="read|write" pattern="SVG" stealth="true"/>
  <policy domain="delegate" rights="execute" pattern="*" stealth="true"/>
  <policy domain="path" rights="read|write" pattern="@*" stealth="true"/>
  <policy domain="filter" rights="read|write|execute" pattern="*" stealth="true"/>
</policymap>
'''
DELEGATES = '<?xml version="1.0" encoding="UTF-8"?>\n<delegatemap>\n  <delegate decode="imbchostile" command="cmd.exe /c echo pwned"/>\n</delegatemap>\n'
for d in [hostile, hostile / '.config/ImageMagick', hostile / 'ImageMagick']:
    (d / 'policy.xml').write_text(PERMISSIVE, encoding='utf-8')
    (d / 'delegates.xml').write_text(DELEGATES, encoding='utf-8')
# Substitute module: a copy of the rgb module renamed svg; if ImageMagick loads it from H, its module path reveals it.
shutil.copy2(runtime / 'modules/coders/IM_MOD_RL_rgb_.dll', hostile / 'coders/IM_MOD_RL_svg_.dll')

advapi = ctypes.WinDLL('advapi32', use_last_error=True)
kernel = ctypes.WinDLL('kernel32', use_last_error=True)
HKEY = wintypes.HANDLE
HKLM, HKCU = HKEY(0xFFFFFFFF80000002), HKEY(0xFFFFFFFF80000001)  # Predefined keys, sign-extended on x64
advapi.RegLoadAppKeyW.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(HKEY), wintypes.DWORD, wintypes.DWORD, wintypes.DWORD]
advapi.RegCreateKeyExW.argtypes = [HKEY, wintypes.LPCWSTR, wintypes.DWORD, wintypes.LPWSTR, wintypes.DWORD, wintypes.DWORD,
                                   ctypes.c_void_p, ctypes.POINTER(HKEY), ctypes.POINTER(wintypes.DWORD)]
advapi.RegSetValueExW.argtypes = [HKEY, wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD]
advapi.RegOverridePredefKey.argtypes = [HKEY, HKEY]
advapi.RegOpenKeyExA.argtypes = [HKEY, ctypes.c_char_p, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(HKEY)]
advapi.RegQueryValueExA.argtypes = [HKEY, ctypes.c_char_p, ctypes.c_void_p, ctypes.POINTER(wintypes.DWORD), ctypes.c_char_p,
                                    ctypes.POINTER(wintypes.DWORD)]


def check(code, what):
    if code != 0:
        raise SystemExit(f'{what} failed: {code}')


hive_file = work / f'private-hive-{os.getpid()}.dat'
app = HKEY()
check(advapi.RegLoadAppKeyW(str(hive_file), ctypes.byref(app), 0xF003F, 0, 0), 'RegLoadAppKeyW')  # KEY_ALL_ACCESS
for subkey in [r'SOFTWARE\ImageMagick\7.1.2\Q:16', r'SOFTWARE\ImageMagick\Current']:
    k = HKEY()
    check(advapi.RegCreateKeyExW(app, subkey, 0, None, 0, 0xF003F, None, ctypes.byref(k), None), 'RegCreateKeyExW')
    for name, value in [('ConfigurePath', hostile), ('CoderModulesPath', hostile / 'coders'), ('FilterModulesPath', hostile / 'filters'),
                        ('LibPath', hostile), ('BinPath', hostile)]:
        data = ctypes.create_unicode_buffer(str(value))
        check(advapi.RegSetValueExW(k, name, 0, 1, data, ctypes.sizeof(data)), 'RegSetValueExW')
check(advapi.RegOverridePredefKey(HKLM, app), 'override HKLM')
check(advapi.RegOverridePredefKey(HKCU, app), 'override HKCU')
# Positive control A: within this process, the ANSI registry API that ImageMagick uses already reads the hostile value.
k = HKEY()
check(advapi.RegOpenKeyExA(HKLM, rb'SOFTWARE\ImageMagick\7.1.2\Q:16', 0, 0x20019, ctypes.byref(k)), 'RegOpenKeyExA(HKLM)')
buf = ctypes.create_string_buffer(1024)
size = wintypes.DWORD(1024)
check(advapi.RegQueryValueExA(k, b'ConfigurePath', None, None, buf, ctypes.byref(size)), 'RegQueryValueExA')
registry_override_effective = buf.value.decode() == str(hostile)

for name in ['MAGICK_CONFIGURE_PATH', 'MAGICK_HOME', 'HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'XDG_CONFIG_HOME']:
    os.environ[name] = str(hostile)
os.environ['MAGICK_CODER_MODULE_PATH'] = str(hostile / 'coders')
os.environ['MAGICK_FILTER_MODULE_PATH'] = str(hostile / 'filters')
os.chdir(hostile)

kernel.LoadLibraryExW.restype = ctypes.c_void_p
kernel.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, ctypes.c_void_p, wintypes.DWORD]
handle = kernel.LoadLibraryExW(str(runtime / 'CORE_RL_MagickCore_.dll'), None, 0x8)  # LOAD_WITH_ALTERED_SEARCH_PATH
if not handle:
    raise SystemExit(f'load failed {ctypes.get_last_error()}')
core = ctypes.CDLL('CORE_RL_MagickCore_.dll', handle=handle)
core.MagickCoreGenesis.argtypes = [ctypes.c_char_p, ctypes.c_int]
core.AcquireExceptionInfo.restype = ctypes.c_void_p
core.GetConfigurePaths.restype = ctypes.c_void_p
core.GetConfigurePaths.argtypes = [ctypes.c_char_p, ctypes.c_void_p]
core.GetNextValueInLinkedList.restype = ctypes.c_void_p
core.GetNextValueInLinkedList.argtypes = [ctypes.c_void_p]
core.IsRightsAuthorized.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_char_p]
core.GetMagickInfo.restype = ctypes.c_void_p
core.GetMagickInfo.argtypes = [ctypes.c_char_p, ctypes.c_void_p]
core.GetDelegateInfo.restype = ctypes.c_void_p
core.GetDelegateInfo.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_void_p]
core.GetMagickResourceLimit.restype = ctypes.c_uint64
core.GetMagickResourceLimit.argtypes = [ctypes.c_int]
core.GetMagickVersion.restype = ctypes.c_char_p
core.GetMagickVersion.argtypes = [ctypes.c_void_p]
core.MagickCoreGenesis(str(runtime / 'magick.exe').encode(), 0)
exc = core.AcquireExceptionInfo()

paths, node = [], core.GetConfigurePaths(b'policy.xml', exc)
while True:
    value = core.GetNextValueInLinkedList(node)
    if not value:
        break
    paths.append(ctypes.string_at(value).decode(errors='replace'))
CODER, DELEGATE, FILTER, PATH, MODULE = 1, 2, 3, 4, 8
READ, WRITE, EXECUTE = 1, 2, 4
rights = {
    'coder RGB write': core.IsRightsAuthorized(CODER, WRITE, b'RGB'),
    'coder SVG read': core.IsRightsAuthorized(CODER, READ, b'SVG'),
    'module SVG read': core.IsRightsAuthorized(MODULE, READ, b'SVG'),
    'delegate execute': core.IsRightsAuthorized(DELEGATE, EXECUTE, b'imbchostile'),
    'path @list read': core.IsRightsAuthorized(PATH, READ, b'@C:\\Windows\\win.ini'),
    'filter execute': core.IsRightsAuthorized(FILTER, EXECUTE, b'analyze'),
    'coder PNG read (expected allowed)': core.IsRightsAuthorized(CODER, READ, b'PNG'),
}
svg_info = core.GetMagickInfo(b'SVG', exc)
rgb_info = core.GetMagickInfo(b'RGB', exc)
kernel.GetModuleHandleW.restype = ctypes.c_void_p
kernel.GetModuleHandleW.argtypes = [wintypes.LPCWSTR]
kernel.GetModuleFileNameW.argtypes = [ctypes.c_void_p, wintypes.LPWSTR, wintypes.DWORD]


def module_path(name):
    h = kernel.GetModuleHandleW(name)
    if not h:
        return None
    b = ctypes.create_unicode_buffer(32768)
    kernel.GetModuleFileNameW(h, b, 32768)
    return b.value


report = {
    'runtime': str(runtime),
    'version': core.GetMagickVersion(None).decode(),
    'registry_override_effective (control A)': registry_override_effective,
    'configure_paths': paths,
    'configure_paths_only_runtime': all(Path(p).resolve() == runtime for p in paths) and len(paths) > 0,
    'rights': rights,
    'disk_limit': core.GetMagickResourceLimit(2),
    'svg_magick_info_loaded': bool(svg_info),
    'hostile_svg_module_loaded_from': module_path('IM_MOD_RL_svg_.dll'),
    'rgb_module_loaded_from': module_path('IM_MOD_RL_rgb_.dll') if rgb_info else None,
    'hostile_delegate_visible': bool(core.GetDelegateInfo(b'imbchostile', None, exc)),
}
hostile_rights = {k: v for k, v in rights.items() if 'expected allowed' not in k}
if expect_permissive:
    report['verdict'] = 'control-detected' if any(hostile_rights.values()) else 'CONTROL-FAILED'
else:
    ok = (registry_override_effective and report['configure_paths_only_runtime'] and not any(hostile_rights.values())
          and rights['coder PNG read (expected allowed)'] and report['hostile_svg_module_loaded_from'] is None
          and not report['hostile_delegate_visible'] and report['disk_limit'] == 8 * 1024 ** 3
          and (report['rgb_module_loaded_from'] or '').lower().startswith(str(runtime).lower()))
    report['verdict'] = 'isolated' if ok else 'NOT-ISOLATED'
print(json.dumps(report, indent=2, ensure_ascii=False))
raise SystemExit(0 if report['verdict'] in ('isolated', 'control-detected') else 1)
