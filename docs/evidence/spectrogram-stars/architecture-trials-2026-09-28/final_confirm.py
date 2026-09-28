from pathlib import Path
import subprocess,sys
A=Path(__file__).parent
runs=[
 ['focused-repeat','base-a,base-b,group3-short100,group3-short75,group3-short50,group3-50','--frames','240','--sizes','3840x2160:4,1920x1080:2','--offset','5'],
 ['focused-synthetic','base-a,base-b,group3-short75,group3-50','--frames','120','--input','synthetic'],
 ['focused-sparse','base-a,base-b,group3-short75,group3-50','--frames','120','--profile','sparse'],
 ['focused-720','base-a,base-b,group3-short75,group3-50','--frames','120','--sizes','926x720:1'],
 ['same-look-final','base-a,base-b,micro,complete','--frames','240'],
]
for run in runs: subprocess.run([sys.executable,str(A/'run.py'),*run],check=True)
