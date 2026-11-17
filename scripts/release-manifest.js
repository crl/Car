import {readFileSync,readdirSync,mkdirSync,copyFileSync,writeFileSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

export function createManifest({version,notes='',publishedAt=new Date().toISOString(),payloads}){
 if(!/^\d+\.\d+\.\d+$/.test(version))throw Error('Only stable versions can be published');
 const platforms={};
 for(const platform of ['windows-x86_64','darwin-aarch64','darwin-x86_64']){
  const payload=payloads[platform];
  if(!payload?.name)throw Error(`Missing ${platform} payload`);
  if(!payload.signature?.trim())throw Error(`Missing ${platform} signature`);
  platforms[platform]={signature:payload.signature.trim(),url:`https://github.com/crl/Car/releases/download/v${version}/${encodeURIComponent(payload.name)}`};
 }
 return {version,notes,pub_date:publishedAt,platforms};
}

function filesIn(folder){return readdirSync(folder,{withFileTypes:true}).flatMap(e=>e.isDirectory()?filesIn(join(folder,e.name)):[join(folder,e.name)])}
export function prepareRelease(source,output){
 const root=fileURLToPath(new URL('../',import.meta.url));
 const pkg=JSON.parse(readFileSync(join(root,'package.json'),'utf8'));
 const config=JSON.parse(readFileSync(join(root,'src-tauri/tauri.conf.json'),'utf8'));
 const cargo=readFileSync(join(root,'src-tauri/Cargo.toml'),'utf8').match(/^version\s*=\s*"([^"]+)"/m)?.[1];
 const version=pkg.version;
 if(config.version!==version||cargo!==version)throw Error('Package versions do not match');
 if(process.env.GITHUB_REF_TYPE==='tag'&&process.env.GITHUB_REF_NAME!==`v${version}`)throw Error('Tag does not match package version');
 const payloads={},copies=[];
 for(const [target,platform,label] of [
  ['x86_64-pc-windows-msvc','windows-x86_64','Windows_x64_setup.exe'],
  ['aarch64-apple-darwin','darwin-aarch64','macOS_arm64.app.tar.gz'],
  ['x86_64-apple-darwin','darwin-x86_64','macOS_x64.app.tar.gz']
 ]){
  const files=filesIn(join(source,`cheguanzhang-${target}`));
  const one=suffix=>{const found=files.filter(f=>f.endsWith(suffix));if(found.length!==1)throw Error(`Expected one ${target} ${suffix}; found ${found.length}`);return found[0]};
  const payload=one(platform.startsWith('darwin')?'.app.tar.gz':'.exe');
  const name=`YunGuanJia_${version}_${label}`;
  payloads[platform]={name,signature:readFileSync(payload+'.sig','utf8')};
  copies.push([payload,name],[payload+'.sig',name+'.sig']);
  if(platform.startsWith('darwin'))copies.push([one('.dmg'),name.replace('.app.tar.gz','.dmg')]);
 }
 const manifest=createManifest({version,notes:readFileSync(join(root,'RELEASE_NOTES.md'),'utf8'),payloads});
 mkdirSync(output,{recursive:true});
 for(const [source,name] of copies)copyFileSync(source,join(output,name));
 writeFileSync(join(output,'latest.json'),JSON.stringify(manifest,null,2)+'\n');
 return manifest;
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
 const manifest=prepareRelease(process.argv[2]||'release-assets',process.argv[3]||'release-out');
 console.log(`Prepared signed release v${manifest.version} for ${Object.keys(manifest.platforms).join(', ')}`);
}
