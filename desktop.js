import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getVersion } from '@tauri-apps/api/app';
import { check } from '@tauri-apps/plugin-updater';
import { createUpdater } from './updater.js';
import { createWebService } from './web-service.js';
let pending=0;
let updater=null;
export const isDesktop=Boolean(window.isTauri);
const secureWeb=location.protocol==='https:';
const web=isDesktop?null:createWebService(window.fetch.bind(window),{
 secure:secureWeb,onSessionExpired:()=>publishAccount(false,{expired:true})
});
export const service={
  async call(command,args){pending++;try{return await (web?web.call(command,args):invoke(command,args))}finally{pending--}},
  async request(path,options={}){
    if(web){
      pending++;
      try{return await web.request(path,options)}finally{pending--}
    }
    const url=new URL(path,'https://local.invalid');
    let value;
    if(url.pathname==='/api/data')value=await this.call('read_data');
    else if(url.pathname==='/api/settings')value=await this.call('read_settings');
    else if(url.pathname==='/api/regions')value=await this.call('query_regions',{parent:url.searchParams.get('parent')||'',q:url.searchParams.get('q')||'',...(url.searchParams.get('all')==='true'?{all:true}:{})});
    else if(options.method==='POST')value=await this.call('save_record',{resource:url.pathname.replace('/api/',''),data:JSON.parse(options.body)});
    else throw Error('不支持的操作');
    return {ok:true,json:async()=>value};
  }
};
export let isAdmin=false;
const accountListeners=new Set();
export function onAccountChange(listener){accountListeners.add(listener);return()=>accountListeners.delete(listener)}
let adminHref=null;
function updateBrowserAdminLink(){
 if(isDesktop||secureWeb)return;
 const link=document.querySelector('#webAdminLink');
 if(!link)return;
 link.hidden=!adminHref;
 if(adminHref){
  const destination=new URL(adminHref);
  destination.hash=location.hash||'#/cars';
  link.href=destination.href;
 }else link.removeAttribute('href');
}
function publishAccount(authenticated,options){
 isAdmin=authenticated;
 document.body.classList.toggle('guest-mode',!isAdmin);
 document.querySelector('#accountName').textContent=isAdmin?'管理员':'游客';
 document.querySelector('#accountAvatar').textContent=isAdmin?'管':'游';
 document.querySelector('#accountHint').textContent=isAdmin?'已登录':'只读模式';
 const button=document.querySelector('#accountButton');
 button.removeAttribute('data-account-pending');
 button.disabled=false;
 updateBrowserAdminLink();
 accountListeners.forEach(listener=>listener(isAdmin,options));
}
export async function refreshAccount(){
 const status=await service.call('account_status');
 if(web&&!secureWeb){
  adminHref=null;
  if(status.admin_url){
   try{
    const url=new URL(status.admin_url);
    if(url.protocol==='https:'&&!url.username&&!url.password)adminHref=url.href;
   }catch{ /* Keep the guest view accessible if the address is unavailable. */ }
  }
 }
 publishAccount((isDesktop||secureWeb)&&Boolean(status.authenticated));
}
let dialogQueue=Promise.resolve();
export function confirmDialog(message){const next=dialogQueue.then(()=>new Promise(resolve=>{const d=document.createElement('dialog');d.className='frontend-dialog frontend-confirm';const p=document.createElement('p');p.textContent=message;d.append(p);for(const [text,value] of [['取消',false],['确定',true]]){const b=document.createElement('button');b.textContent=text;b.onclick=()=>{d.close();d.remove();resolve(value)};d.append(b)}d.addEventListener('cancel',e=>{e.preventDefault();d.remove();resolve(false)});document.body.append(d);d.showModal()}));dialogQueue=next.catch(()=>{});return next}
export async function alertDialog(message){await confirmDialog(message)}
let closing=false;
async function requestExit(){if(closing||pending||updater?.busy||window.frontendBusy?.())return;closing=true;try{if(window.frontendDirty?.()&&!await confirmDialog('还有未保存的内容，确定退出？'))return;await service.call('exit_app')}catch(e){await alertDialog(String(e))}finally{closing=false}}
let acceptedHash=location.hash||'#/cars',routing=false;
window.addEventListener('hashchange',async()=>{
 if(routing){history.replaceState(null,'',acceptedHash);return}
 const next=location.hash;
 // Restore the committed URL without emitting another hashchange while asking.
 history.replaceState(null,'',acceptedHash);
 routing=true;
 try{
  if(pending||updater?.busy||window.frontendBusy?.())return;
  if(window.frontendDirty?.()&&!await confirmDialog('还有未保存的内容，确定放弃修改？'))return;
  if(pending||updater?.busy||window.frontendBusy?.())return;
  if(!window.navigateWorkspace)return;
  acceptedHash=next;history.replaceState(null,'',acceptedHash);window.navigateWorkspace(acceptedHash);updateBrowserAdminLink();
 }finally{routing=false}
});
async function boot(){try{
 if(!isDesktop){
  document.querySelectorAll('[data-desktop-only]').forEach(element=>element.remove());
  document.querySelector('#ledgerLocation').textContent='局域网账本';
  await import('./app.js');
  return;
 }
 await getCurrentWindow().onCloseRequested(async e=>{e.preventDefault();await requestExit()});
 await listen('request-app-close',requestExit);
 const status=await service.call('startup_status');
 if(!status.initialized){await new Promise(resolve=>{const d=document.createElement('dialog');d.className='frontend-dialog frontend-welcome';d.innerHTML='<div class="modal-head"><h2>欢迎使用运管家</h2></div><div class="modal-body"><p>创建本机账本，或导入已有账本。</p><p role="alert"></p></div><div class="modal-footer row"></div>';d.addEventListener('cancel',e=>e.preventDefault());for(const [label,importExisting] of [['创建新账本',false],['导入已有账本',true]]){const b=document.createElement('button');b.textContent=label;b.onclick=async()=>{d.querySelectorAll('button').forEach(x=>x.disabled=true);try{const result=await service.call('initialize_ledger',{importExisting});if(!result?.cancelled){d.remove();resolve()}}catch(e){d.querySelector('[role=alert]').textContent=String(e)}finally{d.querySelectorAll('button').forEach(x=>x.disabled=false)}};d.querySelector('.modal-footer').append(b)}document.body.append(d);d.showModal()})}
 await import('./app.js');
 await initializeUpdates();
}catch(e){const main=document.querySelector('main');main.replaceChildren();const p=document.createElement('p');p.setAttribute('role','alert');p.textContent='启动失败：'+String(e);const b=document.createElement('button');b.textContent='重试';b.onclick=()=>location.reload();main.append(p,b)}}

async function initializeUpdates(){
 updater=createUpdater({document,check,
  prepareInstall:async()=>{
   if(pending||window.frontendBusy?.()){await alertDialog('账本操作尚未完成，请稍后更新。');return false}
   if(window.frontendDirty?.()&&!await confirmDialog('还有未保存的内容。更新将重启运管家，确定放弃这些修改并更新？'))return false;
   if(pending||window.frontendBusy?.())return false;
   await service.call('prepare_update');return true;
  },
  restart:()=>service.call('restart_after_update'),releaseInstall:()=>service.call('cancel_update')
 });
 const settings=document.querySelector('#settingsModal .modal-body');
 const section=document.createElement('section');
 const title=document.createElement('h2');title.textContent='软件更新';
 const version=document.createElement('p');version.className='help';version.textContent=`当前版本 v${await getVersion()}`;
 const button=document.createElement('button');button.type='button';button.textContent='检查更新';
 button.onclick=()=>{updater.open();void updater.checkNow()};
 section.append(title,version,button);settings.append(section);
 void updater.checkNow();
 const timer=setInterval(()=>void updater.checkNow(),6*60*60*1000);
 window.addEventListener('pagehide',()=>{clearInterval(timer);updater.dispose()},{once:true});
}

boot();
