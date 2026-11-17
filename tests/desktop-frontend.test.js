import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {JSDOM} from 'jsdom';
import {createUpdater} from '../updater.js';
const source=readFileSync(new URL('../desktop.js',import.meta.url),'utf8');
function harness(){
 const calls=[],handlers={};let reloads=0;
 const location={hash:'#/cars',reload(){reloads++}};
 const context={URL,Promise,window:{addEventListener:(n,f)=>handlers[n]=f},location,history:{replaceState:(_a,_b,url)=>location.hash=url},invoke:async(command,args)=>{calls.push({command,args});return {items:[]}}};
 vm.createContext(context);
 vm.runInContext(source.replace(/^import .*;$/gm,'').replace(/export /g,'').replace(/boot\(\);\s*$/,'')+'\nglobalThis.api=service;',context);
 return {context,calls,handlers,location,get reloads(){return reloads}};
}
test('update install adapter protects dirty and busy work, then reserves native install',async()=>{
 const h=harness(),dom=new JSDOM('<a class="brand"><svg class="brand-icon"></svg></a><div id="settingsModal"><div class="modal-body"></div></div>');
 let adapter;
 Object.assign(h.context,{document:dom.window.document,check:async()=>null,getVersion:async()=> '0.1.2',setInterval:()=>1,clearInterval:()=>{},createUpdater:options=>{adapter=options;return createUpdater(options)}});
 try{
  await h.context.initializeUpdates();
  assert.match(dom.window.document.querySelector('#settingsModal').textContent,/v0.1.2/);
  h.context.window.frontendDirty=()=>true;
  vm.runInContext('confirmDialog=async()=>false',h.context);
  assert.equal(await adapter.prepareInstall(),false);assert.equal(h.calls.length,0);
  vm.runInContext('confirmDialog=async()=>true',h.context);
  h.context.window.frontendBusy=()=>true;
  assert.equal(await adapter.prepareInstall(),false);assert.equal(h.calls.length,0);
  h.context.window.frontendBusy=()=>false;
  assert.equal(await adapter.prepareInstall(),true);
  assert.equal(h.calls.at(-1).command,'prepare_update');
 }finally{dom.window.close()}
});

test('service translates reads, saves and region filters without HTTP',async()=>{
 const h=harness();await h.context.api.request('/api/data');await h.context.api.request('/api/settings');await h.context.api.request('/api/regions?parent=33&q=杭州');await h.context.api.request('/api/categories/delete',{method:'POST',body:'{"id":7}'});
 assert.deepEqual(JSON.parse(JSON.stringify(h.calls)),[{command:'read_data'},{command:'read_settings'},{command:'query_regions',args:{parent:'33',q:'杭州'}},{command:'save_record',args:{resource:'categories/delete',data:{id:7}}}]);
 assert.doesNotMatch(source,/\bfetch\s*\(|\beval\s*\(|new Function/);
});
test('busy navigation restores committed hash without reload',async()=>{const h=harness();h.context.window.frontendBusy=()=>true;h.location.hash='#/entries';await h.handlers.hashchange();assert.equal(h.location.hash,'#/cars');assert.equal(h.reloads,0)});
test('accepted navigation commits destination before reload',async()=>{const h=harness();h.location.hash='#/cars/7';await h.handlers.hashchange();assert.equal(h.location.hash,'#/cars/7');assert.equal(h.reloads,1)});
test('dirty cancellation retains route; approval commits route',async()=>{const h=harness();h.context.window.frontendDirty=()=>true;vm.runInContext('confirmDialog=async()=>false',h.context);h.location.hash='#/entries';await h.handlers.hashchange();assert.equal(h.location.hash,'#/cars');assert.equal(h.reloads,0);vm.runInContext('confirmDialog=async()=>true',h.context);h.location.hash='#/entries';await h.handlers.hashchange();assert.equal(h.location.hash,'#/entries');assert.equal(h.reloads,1)});
