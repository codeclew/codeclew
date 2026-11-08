"""Offline release-transition tests, including the actual 0.14.7 updater path."""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import runpy
import shutil
import subprocess
import tarfile
import tempfile
import threading
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parent.parent
R=runpy.run_path(str(ROOT/'scripts/refresh_agent_skills.py'))
B=runpy.run_path(str(ROOT/'scripts/build_macos_release.py'))


class QuietHandler(SimpleHTTPRequestHandler):
    def log_message(self, *args): pass


class RefreshTest(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name).resolve();self.install=self.root/'install';self.home=self.root/'home';self.home.mkdir()
        self.old=self.release('0.14.7', legacy=True);self.new=self.release('0.14.8')
        self.patch=patch.object(Path,'home',return_value=self.home);self.patch.start();self.addCleanup(self.patch.stop)

    def release(self, version, legacy=False):
        system='macos' if platform.system()=='Darwin' else 'linux';arch=platform.machine()
        release=self.install/'releases'/f'v{version}-{system}-{arch}-core';source=release/'source'
        (release/'bin').mkdir(parents=True);(source/'skills/codeclew/references').mkdir(parents=True)
        (release/'VERSION').write_text('v'+version+'\n');(release/'PROFILE').write_text('core\n')
        (source/'skills/codeclew/SKILL.md').write_text(f'---\nname: codeclew\nmetadata:\n  version: "{version}"\n---\n# Public fixture\n')
        (source/'skills/codeclew/references/guide.md').write_text('Public instructions '+version)
        for name in ['bootstrap/clew_bootstrap.py','bootstrap/host_resources.py','scripts/install_agent_skill.py','packaging/macos/upgrade','site/install.sh']:
            path=source/name;path.parent.mkdir(parents=True,exist_ok=True)
            if legacy and name in ['scripts/install_agent_skill.py','packaging/macos/upgrade','site/install.sh']:
                path.write_bytes(subprocess.run(['git','show','v0.14.7:'+name],cwd=ROOT,capture_output=True,check=True).stdout)
            else:shutil.copyfile(ROOT/name,path)
        launcher=release/'bin/clew'
        launcher.write_bytes(subprocess.run(['git','show','v0.14.7:packaging/macos/clew'],cwd=ROOT,capture_output=True,check=True).stdout if legacy else (ROOT/'packaging/macos/clew').read_bytes());launcher.chmod(0o500)
        if not legacy:shutil.copyfile(ROOT/'scripts/refresh_agent_skills.py',source/'scripts/refresh_agent_skills.py')
        runtime=source/'clew';runtime.write_text('#!/bin/sh\ncase "$1" in\n--version) printf "clew '+version+'\\n";;\ncapabilities) printf \'{"status":"PILOT_READY"}\\n\';;\n*) exit 2;;\nesac\n');runtime.chmod(0o500)
        rows=[]
        for path in sorted(source.rglob('*')):
            if path.is_file():
                mode=0o111 if path.stat().st_mode&0o111 else 0;path.chmod(0o500 if mode else 0o400)
                rows.append({'mode':mode,'path':path.relative_to(source).as_posix(),'sha256':B['sha256'](path.read_bytes()),'size':path.stat().st_size})
        revision=('a' if legacy else 'b')*40
        manifest={'schema':'codeclew-release-source/1.0','files':rows,'sourceRevision':revision,'sourceTree':'c'*40,'manifestDigest':''}
        manifest['manifestDigest']=B['sha256'](R['canonical'](manifest));(source/'release-source.json').write_bytes(R['canonical'](manifest)+b'\n');(source/'release-source.json').chmod(0o400)
        seed={'mode':'RELEASE','sourceRevision':revision,'sourceTree':'c'*40,'sourcePayloadDigest':manifest['manifestDigest']}
        seed['seedDigest']=B['sha256'](R['canonical'](seed));sp=release/'seed'/('release-N-'+revision)/'seed.json';sp.parent.mkdir(parents=True);sp.write_bytes(R['canonical'](seed)+b'\n');sp.chmod(0o400)
        return release

    def install_copy(self, path=None, receipt=False, pinned=False):
        path=path or self.home/'.agents/skills/codeclew'
        rows,digest,_=R['verified_skill'](self.old)
        R['INSTALL']['install_one'](path.parent,rows,False)
        if receipt:R['record'](self.old/'source',[{'path':str(path)}],digest,pinned)
        return path

    def test_legacy_exact_official_content_refreshes_once(self):
        path=self.install_copy();messages=R['refresh'](self.new)
        self.assertIn('0.14.8',(path/'SKILL.md').read_text());self.assertIn('legacy official-content match',messages[0])
        state=(self.install/'skill-installs.json').read_bytes();self.assertEqual(R['refresh'](self.new),[])
        self.assertEqual((self.install/'skill-installs.json').read_bytes(),state)

    def test_added_deleted_modified_unknown_and_pinned_copies_are_preserved(self):
        for kind in ['added','deleted','modified','unknown','pinned']:
            with self.subTest(kind=kind):
                path=self.home/'.claude/skills/codeclew';self.install_copy(path,receipt=kind=='pinned',pinned=kind=='pinned')
                if kind=='added':(path/'local.md').write_text('Local addition')
                if kind=='deleted':(path/'references/guide.md').unlink()
                if kind=='modified':(path/'references/guide.md').write_text('Local change')
                if kind=='unknown':(path/'SKILL.md').write_text('# A custom skill')
                before={str(p.relative_to(path)):p.read_bytes() for p in path.rglob('*') if p.is_file()}
                self.assertTrue(any('Preserved' in m for m in R['refresh'](self.new)))
                self.assertEqual(before,{str(p.relative_to(path)):p.read_bytes() for p in path.rglob('*') if p.is_file()})
                shutil.rmtree(path);(self.install/'skill-release.json').unlink()

    def test_fresh_activation_does_not_create_absent_skills(self):
        self.assertEqual(R['refresh'](self.new),[])
        self.assertFalse((self.home/'.agents').exists());self.assertFalse((self.home/'.claude').exists())

    def test_isolated_prefix_does_not_adopt_or_refresh_global_legacy_copy(self):
        path=self.install_copy();before=(path/'SKILL.md').read_bytes();shutil.rmtree(self.old)
        self.assertEqual(R['refresh'](self.new),[])
        self.assertEqual((path/'SKILL.md').read_bytes(),before)
        self.assertEqual(R['receipts'](self.install)['copies'],{})

    def test_explicit_installed_command_records_pinned_destination(self):
        destination=self.root/'explicit-skills'
        result=subprocess.run([str(self.new/'bin/clew'),'skill','install','--destination',str(destination),'--pin'],capture_output=True,text=True,timeout=20)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertTrue(R['receipts'](self.install)['copies'][str(destination/'codeclew')]['pinned'])

    def test_explicit_recorded_project_updates_but_invalid_receipt_paths_do_not(self):
        project=self.root/'project/.agents/skills/codeclew';self.install_copy(project,receipt=True)
        R['refresh'](self.new);self.assertIn('0.14.8',(project/'SKILL.md').read_text())
        (self.install/'skill-release.json').unlink()
        outside=self.install_copy(self.root/'other/codeclew')
        value=R['receipts'](self.install);value['copies'][str(self.root/'unused/../other/codeclew')]={'release':str(self.old),'digest':R['verified_skill'](self.old)[1],'pinned':False}
        R['write_state'](self.install/'skill-installs.json',value);R['refresh'](self.new)
        self.assertIn('0.14.7',(outside/'SKILL.md').read_text())

    def test_unverified_previous_bundle_cannot_supply_a_legacy_match(self):
        path=self.install_copy();source=self.old/'source/skills/codeclew/SKILL.md';source.chmod(0o600);source.write_text('Changed sealed source');source.chmod(0o400)
        self.assertTrue(any('Preserved' in m for m in R['refresh'](self.new)))
        self.assertIn('0.14.7',(path/'SKILL.md').read_text())

    def test_actual_0147_upgrade_activates_new_launcher_and_refreshes_existing_skill(self):
        path=self.install_copy();bin_dir=self.root/'bin';bin_dir.mkdir();entry=bin_dir/'clew';entry.symlink_to(self.old/'bin/clew')
        web=self.root/'www';assets=web/'releases/download/v0.14.8';assets.mkdir(parents=True)
        system='macos' if platform.system()=='Darwin' else 'linux';archive=assets/f'codeclew-{system}-{platform.machine()}.tar.gz'
        with tarfile.open(archive,'w:gz') as out:out.add(self.new,arcname='codeclew')
        (assets/(archive.name+'.sha256')).write_text(B['file_sha256'](archive)+'  '+archive.name+'\n')
        (web/'latest.json').write_text('{"tag_name":"v0.14.8"}')
        shutil.rmtree(self.new)
        server=ThreadingHTTPServer(('127.0.0.1',0),partial(QuietHandler,directory=str(web)));thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            env=dict(os.environ,HOME=str(self.home),CODECLEW_ALLOW_INSECURE_DOWNLOAD='1',CODECLEW_RELEASE_API=f'http://127.0.0.1:{server.server_port}/latest.json',CODECLEW_RELEASE_BASE=f'http://127.0.0.1:{server.server_port}/releases')
            result=subprocess.run([str(entry),'upgrade'],env=env,capture_output=True,text=True,timeout=45)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertIn('0.14.8',(path/'SKILL.md').read_text());self.assertIn('legacy official-content match',result.stderr)
            second=subprocess.run([str(entry),'capabilities'],env=env,capture_output=True,text=True,timeout=20)
            self.assertEqual(second.returncode,0,second.stderr);self.assertNotIn('Updated',second.stderr);self.assertEqual(json.loads(second.stdout)['status'],'PILOT_READY')
        finally:server.shutdown();server.server_close();thread.join()


if __name__=='__main__':unittest.main()
