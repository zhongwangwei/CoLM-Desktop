"""Small orchestration checks, without model datasets or an SSH server."""
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).with_name('remote_cases.py')

class RemoteCreation(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        self.cli = self.root / 'cli'
        self.cli.write_text('''#!/usr/bin/env python3
import json, pathlib, sys
args = sys.argv[1:]
def get(k): return args[args.index(k)+1]
if args[0] == 'scan':
 print(json.dumps({'sites': [{'path': get('--dir')+'/site.nc'}]}))
elif args[0] == 'new':
 out = pathlib.Path(get('--out'))
 if get('--site') == 'fail': raise RuntimeError('bad station')
 (out/'case.nml').write_text("&nl_colm\\n DEF_file_site = '/data/site.nc'\\n DEF_forcing = '"+str(out/"forcing.nml")+"'\\n/\\n")
 (out/'forcing.nml').write_text("&forcing\\n path = '/data/met'\\n/\\n")
''')
        with self.cli.open('a') as stream:
            stream.write("\nif args[0] == 'new':\n external=pathlib.Path(get('--site')+'.nml')\n if external.is_file():\n  with (out/'case.nml').open('a') as f: f.write(\"&extra\\n ref = '\"+str(external).replace(\"'\",\"''\")+\"'\\n/\\n\")\n")
        self.cli.chmod(0o700)

    def tearDown(self): self.tmp.cleanup()

    def run_action(self, action, request, success=True, fail_replace=False):
        if action == 'create': request.setdefault('token', 'a' * 64)
        launcher = [sys.executable, str(SCRIPT)]
        if fail_replace:
            launcher = [sys.executable, '-c', "import os,runpy,sys; original=os.replace; count=[0]\ndef replace(a,b):\n count[0]+=1\n if count[0]==2: raise OSError('injected second replacement failure')\n return original(a,b)\nos.replace=replace; sys.argv=sys.argv[1:]; runpy.run_path(sys.argv[0],run_name='__main__')",str(SCRIPT)]
        p = subprocess.run(launcher + [json.dumps({'cli':str(self.cli),'action':action,'request':request})], capture_output=True, text=True)
        self.assertEqual(p.returncode == 0, success, p.stderr)
        return json.loads(p.stdout) if success else p.stderr

    def test_creation_publish_and_no_overwrite(self):
        out = self.root/'projects'/'my case'
        req = {'out':str(out),'kind':'site','site':'/data/site.nc'}
        result = self.run_action('create',req)
        self.assertEqual(set(result['files']), {'case.nml','forcing.nml'})
        self.assertIn('/data/site.nc', result['files']['case.nml'])
        recovered = self.run_action('create',req)
        self.assertEqual(recovered, result)
        self.run_action('create',dict(req, token='b'*64),False)
        self.assertTrue((out/'case.nml').is_file())
        self.run_action('publish',{'out':str(out),'token':'wrong','files':result['files']},False)
        self.run_action('publish',{'out':str(out),'token':result['token'],'files':result['files']})
        self.assertEqual(json.loads((out/'.colm-desktop-server.json').read_text())['remote_directory'],str(out))
        self.run_action('abort',{'out':str(out),'token':result['token']},False)
        self.assertTrue(out.is_dir())

    def test_failure_removes_only_owned_destination(self):
        out = self.root/'project'/'bad'
        keep = self.root/'project'/'input.nc'
        keep.parent.mkdir()
        keep.write_text('input')
        self.run_action('create',{'out':str(out),'kind':'site','site':'fail'},False)
        self.assertFalse(out.exists())
        self.assertEqual(keep.read_text(),'input')

    def test_abort_and_publish_path_traversal(self):
        out = self.root/'case'
        result = self.run_action('create',{'out':str(out),'kind':'site','site':'/data/site.nc'})
        self.run_action('publish',{'out':str(out),'token':result['token'],'files':{'case.nml':'a','../outside.nml':'x'}},False)
        self.assertFalse((self.root/'outside.nml').exists())
        self.run_action('abort',{'out':str(out),'token':result['token']})
        self.assertFalse(out.exists())

    def test_missing_runtime_input_prevents_publish_and_sync_retains_owner(self):
        out = self.root/'case'
        result = self.run_action('create',{'out':str(out),'kind':'site','site':'/data/site.nc'})
        request = {'out':str(out),'token':result['token'],'files':result['files'],'checks':[{'path':str(self.root/'absent.nc'),'directory':False}]}
        self.run_action('publish',request,False)
        self.assertTrue((out/'.colm-desktop-creation.json').is_file())
        request['checks'] = []
        self.run_action('publish',request)
        request['files']['case.nml'] = '&nl_colm\n new = 1\n/\n'
        self.run_action('sync',request)
        self.assertEqual((out/'case.nml').read_text(),request['files']['case.nml'])
        (out/'.colm-desktop-server.json').write_text('{}')
        self.run_action('sync',request,False)

    def test_interrupted_owned_creation_resumes_but_other_token_cannot(self):
        out = self.root/'interrupted'
        out.mkdir()
        (out/'.colm-desktop-creation.json').write_text(json.dumps({'token':'a'*64}))
        (out/'partial.nc').write_text('incomplete')
        request = {'out':str(out),'kind':'site','site':'/data/site.nc','token':'b'*64}
        self.run_action('create',request,False)
        self.assertTrue((out/'partial.nc').exists())
        request['token'] = 'a'*64
        result = self.run_action('create',request)
        self.assertFalse((out/'partial.nc').exists())
        self.assertEqual(result['token'],'a'*64)
        self.assertEqual(self.run_action('create',request),result)

    def test_second_file_publish_failure_restores_every_old_file(self):
        out = self.root/'case'
        result = self.run_action('create',{'out':str(out),'kind':'site','site':'/data/site.nc'})
        request = {'out':str(out),'token':result['token'],'files':result['files']}
        self.run_action('publish',request)
        old = dict(result['files'])
        request['files'] = {'case.nml':'new case','forcing.nml':'new forcing'}
        self.run_action('sync',request,False,fail_replace=True)
        for name, text in old.items():
            self.assertEqual((out/name).read_text(),text)
        self.assertEqual(list(out.glob('.backup-*')),[])

    def test_apostrophe_external_references_and_cycles_are_cached_once(self):
        external = self.root / "O'Brien"
        external.mkdir()
        first, second = external/'site.nc.nml', external/'second.nml'
        def nml(target): return "! don't rewrite this comment\n&extra\n ref = '" + str(target).replace("'", "''") + "'\n/\n"
        first.write_text(nml(second))
        second.write_text(nml(first))
        out = self.root/'case'
        result = self.run_action('create',{'out':str(out),'kind':'site','site':str(external/'site.nc')})
        self.assertEqual(len(result['files']),4)
        self.assertNotIn("O''Brien", ''.join(result['files'].values()))
        self.assertIn(str(out/'second.nml'),result['files']['site.nc.nml'])
        self.assertIn(str(out/'site.nc.nml'),result['files']['second.nml'])

    def test_corrected_retry_requires_absent_or_incomplete_owned_destination(self):
        out = self.root/'case'
        self.run_action('create',{'out':str(out),'kind':'site','site':'fail'},False)
        self.run_action('abort',{'out':str(out),'token':'a'*64,'incomplete_only':True})
        result = self.run_action('create',{'out':str(out),'kind':'site','site':'/corrected.nc','token':'b'*64})
        self.run_action('abort',{'out':str(out),'token':'b'*64,'incomplete_only':True},False)
        self.assertTrue((out/'case.nml').exists())
        self.assertEqual(self.run_action('create',{'out':str(out),'kind':'site','site':'/corrected.nc','token':'b'*64}),result)
        self.run_action('abort',{'out':str(out),'token':'b'*64})
        out.mkdir()
        (out/'.colm-desktop-creation.json').write_text(json.dumps({'token':'c'*64}))
        self.run_action('abort',{'out':str(out),'token':'wrong','incomplete_only':True},False)
        self.assertTrue(out.is_dir())
        self.run_action('abort',{'out':str(out),'token':'c'*64,'incomplete_only':True})
        self.assertFalse(out.exists())

    def test_scan_keeps_server_paths(self):
        self.assertEqual(self.run_action('scan',{'dir':'/server/sites','quick':True})['sites'][0]['path'],'/server/sites/site.nc')

if __name__ == '__main__': unittest.main()
