import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';

async function setup(overrides={}){
 const {createUpdater}=await import('../updater.js');
 const dom=new JSDOM('<a class="brand" href="#/cars"><svg class="brand-icon"></svg>运管家</a>');
 const w=dom.window,events=[];
 w.HTMLDialogElement.prototype.showModal=function(){this.open=true};
 w.HTMLDialogElement.prototype.close=function(){this.open=false};
 const update={version:'0.1.3',body:'修复 <script>不能执行</script>',download:async cb=>{
  events.push('download');cb({event:'Started',data:{contentLength:100}});cb({event:'Progress',data:{chunkLength:100}});
 },install:async()=>events.push('install'),close:async()=>events.push('dispose')};
 const api=createUpdater({document:w.document,check:async()=>update,prepareInstall:async()=>true,
  restart:async()=>events.push('restart'),releaseInstall:async()=>events.push('release'),...overrides});
 return {dom,api,events,update,d:w.document,close(){api.dispose();w.close()}};
}

test('new release shows brand dot and notes; click does not install without confirmation',async()=>{
 const h=await setup();try{
  assert.equal(h.d.querySelector('.update-dot').hidden,true);
  await h.api.checkNow();assert.equal(h.d.querySelector('.update-dot').hidden,false);
  h.d.querySelector('.brand').click();
  assert.equal(h.d.querySelector('#updateDialog').open,true);
  assert.match(h.d.querySelector('#updateNotes').textContent,/修复 <script>/);
  assert.equal(h.d.querySelector('#updateNotes script'),null);
  assert.deepEqual(h.events,[]);
  await h.api.startUpdate();assert.deepEqual(h.events,['download','install','restart','release']);
  assert.equal(h.d.querySelector('#updateProgress').value,100);
 }finally{h.close()}
});

test('background failures stay silent and manual failures allow retry',async()=>{
 let attempts=0;
 const h=await setup({check:async()=>{if(++attempts===1)throw Error('offline');return null}});
 try{
  await h.api.checkNow();assert.equal(h.d.querySelector('#updateDialog').open,false);
  assert.equal(h.d.querySelector('.update-dot').hidden,true);
  h.api.open();assert.match(h.d.querySelector('#updateStatus').textContent,/失败/);
  await h.api.checkNow();assert.match(h.d.querySelector('#updateStatus').textContent,/最新/);
 }finally{h.close()}
});

test('cancelled dirty guard prevents installation and keeps update available',async()=>{
 const h=await setup({prepareInstall:async()=>false});try{
  await h.api.checkNow();await h.api.startUpdate();
  assert.deepEqual(h.events,['download']);assert.equal(h.api.busy,false);
  assert.equal(h.d.querySelector('.update-dot').hidden,false);
 }finally{h.close()}
});

test('install failure releases native guard and concurrent clicks do not double install',async()=>{
 let resume;const waiting=new Promise(r=>resume=r);let count=0;
 const h=await setup();h.update.install=async()=>{count++;await waiting;throw Error('denied')};
 try{
  await h.api.checkNow();const work=h.api.startUpdate();await Promise.resolve();await Promise.resolve();
  await h.api.startUpdate();resume();await work;
  assert.equal(count,1);assert.equal(h.api.busy,false);
  assert.ok(h.events.includes('release'));assert.match(h.d.querySelector('#updateStatus').textContent,/失败/);
  assert.equal(h.d.querySelector('#updateAction').disabled,false);
 }finally{h.close()}
});
