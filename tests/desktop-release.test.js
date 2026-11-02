import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,rmSync,existsSync} from 'node:fs';
import {join} from 'node:path';
import {tmpdir} from 'node:os';

const payloads={
 'windows-x86_64':{name:'YunGuanJia_0.1.2_Windows_x64_setup.exe',signature:'windows signature'},
 'darwin-aarch64':{name:'YunGuanJia_0.1.2_macOS_arm64.app.tar.gz',signature:'arm signature'},
 'darwin-x86_64':{name:'YunGuanJia_0.1.2_macOS_x64.app.tar.gz',signature:'intel signature'}
};
test('release manifest maps signed payloads to all three Tauri platform keys',async()=>{
 const {createManifest}=await import('../scripts/release-manifest.js');
 const m=createManifest({version:'0.1.2',notes:'版本说明',publishedAt:'2026-11-02T00:00:00Z',payloads});
 assert.equal(m.version,'0.1.2');assert.equal(m.notes,'版本说明');
 assert.equal(m.platforms['darwin-aarch64'].signature,'arm signature');
 assert.equal(m.platforms['windows-x86_64'].url,'https://github.com/crl/Car/releases/download/v0.1.2/YunGuanJia_0.1.2_Windows_x64_setup.exe');
 assert.equal(Object.keys(m.platforms).length,3);
});
test('incomplete or prerelease manifests cannot become the stable update feed',async()=>{
 const {createManifest}=await import('../scripts/release-manifest.js');
 assert.throws(()=>createManifest({version:'0.1.2-beta.1',payloads}),/stable/);
 assert.throws(()=>createManifest({version:'0.1.2',payloads:{...payloads,'darwin-x86_64':null}}),/darwin-x86_64/);
 assert.throws(()=>createManifest({version:'0.1.2',payloads:{...payloads,'windows-x86_64':{name:'x.exe',signature:''}}}),/signature/);
});

test('downloaded CI artifacts produce complete release assets; missing signatures block publication',async()=>{
 const {prepareRelease}=await import('../scripts/release-manifest.js');
 const root=mkdtempSync(join(tmpdir(),'car-release-'));
 try{
  const source=join(root,'artifacts');
  for(const [target,suffix] of [['x86_64-pc-windows-msvc','.exe'],['aarch64-apple-darwin','.app.tar.gz'],['x86_64-apple-darwin','.app.tar.gz']]){
   const folder=join(source,`cheguanzhang-${target}`,'bundle');mkdirSync(folder,{recursive:true});
   writeFileSync(join(folder,'app'+suffix),'package bytes');writeFileSync(join(folder,'app'+suffix+'.sig'),'signed bytes');
   if(suffix!=='.exe')writeFileSync(join(folder,'app.dmg'),'disk image');
  }
  const output=join(root,'ready'),manifest=prepareRelease(source,output);
  assert.deepEqual(JSON.parse(readFileSync(join(output,'latest.json'),'utf8')),manifest);
  const asset=decodeURIComponent(manifest.platforms['windows-x86_64'].url.split('/').at(-1));
  assert.equal(readFileSync(join(output,asset),'utf8'),'package bytes');
  assert.equal(readFileSync(join(output,asset+'.sig'),'utf8'),'signed bytes');
  rmSync(join(source,'cheguanzhang-x86_64-apple-darwin/bundle/app.app.tar.gz.sig'));
  assert.throws(()=>prepareRelease(source,join(root,'incomplete')),/ENOENT/);
  assert.equal(existsSync(join(root,'incomplete/latest.json')),false);
 }finally{rmSync(root,{recursive:true,force:true})}
});
