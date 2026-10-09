"""Compile actual production boundary bodies against fake unrelated driver dependencies.

No SteamVR runtime loads/driver registration. The real SDK's IVRServerDriverHost is
implemented by the fake host. Extraction hashes are printed for auditability. Full
streamer compilation is a separate gate; this harness is not a DLL/HIL test.
"""
from pathlib import Path
import hashlib, json, os, subprocess, sys
ROOT=Path(__file__).resolve().parents[4]
CPP=ROOT/'alvr/server_openvr/cpp'
OUT=ROOT/'target/hmd-authority-cpp-test'
OUT.mkdir(parents=True,exist_ok=True)
def body(file, signature):
    text=(CPP/file).read_text()
    start=text.index(signature); opening=text.index('{',start); depth=1; end=opening+1
    while depth:
        if text[end]=='{': depth+=1
        if text[end]=='}': depth-=1
        end+=1
    result=text[start:end]
    print(json.dumps({'file':file,'signature':signature,'body_sha256':hashlib.sha256(result.encode()).hexdigest()}))
    return result
parts=[body('alvr_server/TrackedDevice.cpp','void TrackedDevice::submit_pose('),
       body('alvr_server/HMD.cpp','void Hmd::OnPoseUpdated('),
       body('alvr_server/alvr_server.cpp','void SetTracking(')]
test=(CPP/'tools/hmd_authority_test.cpp').read_text()
source=OUT/'harness.cpp'
source.write_text(test+'\n'+'\n'.join(parts),newline='\n')
if os.name!='nt': raise SystemExit('This harness currently requires MSVC on Windows')
vs=Path(os.environ.get('MONAKA_TEST_VS_ROOT','C:/Program Files/Microsoft Visual Studio/18/Insiders'))
msvc=sorted((vs/'VC/Tools/MSVC').iterdir())[-1]
sdk=Path(os.environ.get('MONAKA_TEST_SDK_ROOT','C:/Program Files (x86)/Windows Kits/10'))
sdk_version=sorted((sdk/'Include').iterdir())[-1].name
env=os.environ.copy()
env['PATH']=str(msvc/'bin/Hostx64/x64')+';'+str(sdk/'bin'/sdk_version/'x64')+';'+env['PATH']
env['INCLUDE']=';'.join(map(str,[msvc/'include',*[sdk/'Include'/sdk_version/p for p in ['ucrt','shared','um','winrt']]]))
env['LIB']=';'.join(map(str,[msvc/'lib/x64',sdk/'Lib'/sdk_version/'ucrt/x64',sdk/'Lib'/sdk_version/'um/x64']))
exe=OUT/'harness.exe'
subprocess.run([str(msvc/'bin/Hostx64/x64/cl.exe'),'/nologo','/std:c++17','/EHsc','/W4',f'/I{CPP}',f'/I{ROOT}/openvr/headers',
    str(source),f'/Fe{exe}',f'/Fo{OUT}/harness.obj'],env=env,check=True)
subprocess.run([str(exe),*sys.argv[1:]],check=True,env=env)
