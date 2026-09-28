from pathlib import Path
import subprocess,sys
A=Path(__file__).parent
runs=[
 ['screen4-combinations','base-a,base-b,group3-50,group3-short75,group3-short50,hybrid-24','--frames','240'],
 ['confirm-main-a','base-a,base-b,direct4,residual4,group5-75,group5-50','--frames','360'],
 ['confirm-main-b','base-a,base-b,direct4,residual4,group5-75,group5-50','--frames','360','--sizes','3840x2160:4,1920x1080:2','--offset','5'],
 ['confirm-main-synthetic','base-a,base-b,direct4,residual4,group5-75,group5-50','--frames','240','--input','synthetic'],
 ['confirm-main-sparse','base-a,base-b,direct4,residual4,group5-75,group5-50','--frames','120','--profile','sparse'],
 ['confirm-main-720','base-a,base-b,direct4,residual4,group5-75,group5-50','--frames','240','--sizes','926x720:1'],
 ['same-look-a','base-a,base-b,micro,complete','--frames','240'],
 ['same-look-b','base-a,base-b,micro,complete','--frames','240','--sizes','3840x2160:4,1920x1080:2','--offset','11'],
 ['confirm-hybrids','base-a,base-b,hybrid-8,hybrid-16,hybrid-24,group3-50','--frames','240'],
]
for run in runs:
 subprocess.run([sys.executable,str(A/'run.py'),*run],check=True)
