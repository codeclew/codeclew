'use strict';
const {test}=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs');const os=require('node:os');const path=require('node:path');const {spawnSync}=require('node:child_process');
const script=path.resolve('crates/clew/assets/documentation/docusaurus/pack-evidence.mjs');
const id=n=>'sha256:'+String(n).repeat(64),ref=n=>({$clewRef:id(n)});
function fixture(){
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'clew-doc-packs-')),data=path.join(root,'static/clew-data');
 fs.mkdirSync(path.join(data,'objects'),{recursive:true});fs.mkdirSync(path.join(data,'widgets'));
 function object(n,value,raw){fs.writeFileSync(path.join(data,'objects',id(n).slice(7)+'.json'),raw||JSON.stringify({schema:'codeclew-export-object/1.0',identity:id(n),value}));}
 object(1,{source:'shared'});object(2,{source:'only a'});object(3,{source:'only b'});object(4,{shared:ref(1),own:ref(2),literal:{$clewLiteral:{$clewRef:'not a reference'}}});object(5,{shared:ref(1),own:ref(3)});
 function widget(name,n){fs.writeFileSync(path.join(data,'widgets',name+'.json'),JSON.stringify({schema:'codeclew-normalized-data/1.0',value:ref(n)}));}
 widget('a',4);widget('b',5);
 return {root,data,object,widget,run:()=>spawnSync(process.execPath,[script,root],{encoding:'utf8'}),close:()=>fs.rmSync(root,{recursive:true,force:true})};
}
test('shared packs preserve each closure with no repeated or unrelated object and deterministic output',()=>{
 const f=fixture();try{
  const run=f.run();assert.equal(run.status,0,run.stderr);const out=path.join(f.root,'static/clew-reader-packs'),manifest=JSON.parse(fs.readFileSync(path.join(out,'manifest.json')));
  const all=new Set();for(const name of fs.readdirSync(out).filter(n=>n.startsWith('pack-'))){for(const key of Object.keys(JSON.parse(fs.readFileSync(path.join(out,name))).objects)){assert.ok(!all.has(key),'duplicated object '+key);all.add(key);}}
  assert.equal(all.size,5);
  for(const [name,expected]of [['a',[1,2,4]],['b',[1,3,5]]]){
   const widget=manifest.widgets['/clew-data/widgets/'+name+'.json'];assert.equal(widget.packs.length,2);const actual=widget.packs.flatMap(p=>Object.keys(JSON.parse(fs.readFileSync(path.join(f.root,'static',p.slice(1)))).objects));assert.deepEqual(actual.sort(),expected.map(id).sort());
  }
  const before=Object.fromEntries(fs.readdirSync(out).map(n=>[n,fs.readFileSync(path.join(out,n),'utf8')]));fs.writeFileSync(path.join(out,'user-note.txt'),'preserve');assert.equal(f.run().status,0);
  assert.equal(fs.readFileSync(path.join(out,'user-note.txt'),'utf8'),'preserve');for(const [name,value]of Object.entries(before))assert.equal(fs.readFileSync(path.join(out,name),'utf8'),value);
 }finally{f.close();}
});
test('transport retains original numeric spelling and rejects missing or cyclic data before replacement',()=>{
 const f=fixture();try{
  const raw='{"schema":"codeclew-export-object/1.0","identity":"'+id(1)+'","value":{"number":1.0,"text":"λ\\r\\n"}}';f.object(1,null,raw);assert.equal(f.run().status,0);
  const out=path.join(f.root,'static/clew-reader-packs'),before=fs.readFileSync(path.join(out,'manifest.json'),'utf8');assert.ok(fs.readdirSync(out).filter(n=>n.startsWith('pack-')).some(n=>fs.readFileSync(path.join(out,n),'utf8').includes(raw)));
  f.object(1,ref(4));assert.notEqual(f.run().status,0);assert.equal(fs.readFileSync(path.join(out,'manifest.json'),'utf8'),before);
  fs.unlinkSync(path.join(f.data,'objects',id(1).slice(7)+'.json'));assert.notEqual(f.run().status,0);assert.equal(fs.readFileSync(path.join(out,'manifest.json'),'utf8'),before);
 }finally{f.close();}
});
