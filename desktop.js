import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
let pending=0;
export const service={
  async call(command,args){pending++;try{return await invoke(command,args)}finally{pending--}},
  async request(path,options={}){
    const url=new URL(path,'https://local.invalid');
    let value;
    if(url.pathname==='/api/data')value=await this.call('read_data');
    else if(url.pathname==='/api/settings')value=await this.call('read_settings');
    else if(url.pathname==='/api/regions')value=await this.call('query_regions',{parent:url.searchParams.get('parent')||'',q:url.searchParams.get('q')||''});
    else if(options.method==='POST')value=await this.call('save_record',{resource:url.pathname.replace('/api/',''),data:JSON.parse(options.body)});
    else throw Error('不支持的操作');
    return {ok:true,json:async()=>value};
  }
};
let dialogQueue=Promise.resolve();
export function confirmDialog(message){const next=dialogQueue.then(()=>new Promise(resolve=>{const d=document.createElement('dialog');d.className='frontend-dialog frontend-confirm';const p=document.createElement('p');p.textContent=message;d.append(p);for(const [text,value] of [['取消',false],['确定',true]]){const b=document.createElement('button');b.textContent=text;b.onclick=()=>{d.close();d.remove();resolve(value)};d.append(b)}d.addEventListener('cancel',e=>{e.preventDefault();d.remove();resolve(false)});document.body.append(d);d.showModal()}));dialogQueue=next.catch(()=>{});return next}
export async function alertDialog(message){await confirmDialog(message)}
let closing=false;
async function requestExit(){if(closing||pending||window.frontendBusy?.())return;closing=true;try{if(window.frontendDirty?.()&&!await confirmDialog('还有未保存的内容，确定退出？'))return;await service.call('exit_app')}catch(e){await alertDialog(String(e))}finally{closing=false}}
let acceptedHash=location.hash||'#/cars',routing=false;
window.addEventListener('hashchange',async()=>{
 if(routing){history.replaceState(null,'',acceptedHash);return}
 const next=location.hash;
 // Restore the committed URL without emitting another hashchange while asking.
 history.replaceState(null,'',acceptedHash);
 routing=true;
 try{
  if(pending||window.frontendBusy?.())return;
  if(window.frontendDirty?.()&&!await confirmDialog('还有未保存的内容，确定放弃修改？'))return;
  if(pending||window.frontendBusy?.())return;
  acceptedHash=next;history.replaceState(null,'',acceptedHash);location.reload();
 }finally{routing=false}
});
async function boot(){try{
 await getCurrentWindow().onCloseRequested(async e=>{e.preventDefault();await requestExit()});
 await listen('request-app-close',requestExit);
 const status=await service.call('startup_status');
 if(!status.initialized){await new Promise(resolve=>{const d=document.createElement('dialog');d.className='frontend-dialog frontend-welcome';d.innerHTML='<div class="modal-head"><h2>欢迎使用运管家</h2></div><div class="modal-body"><p>创建本机账本，或导入已有账本。</p><p role="alert"></p></div><div class="modal-footer row"></div>';d.addEventListener('cancel',e=>e.preventDefault());for(const [label,importExisting] of [['创建新账本',false],['导入已有账本',true]]){const b=document.createElement('button');b.textContent=label;b.onclick=async()=>{d.querySelectorAll('button').forEach(x=>x.disabled=true);try{const result=await service.call('initialize_ledger',{importExisting});if(!result?.cancelled){d.remove();resolve()}}catch(e){d.querySelector('[role=alert]').textContent=String(e)}finally{d.querySelectorAll('button').forEach(x=>x.disabled=false)}};d.querySelector('.modal-footer').append(b)}document.body.append(d);d.showModal()})}
 await import('./app.js');
}catch(e){const main=document.querySelector('main');main.replaceChildren();const p=document.createElement('p');p.setAttribute('role','alert');p.textContent='启动失败：'+String(e);const b=document.createElement('button');b.textContent='重试';b.onclick=()=>location.reload();main.append(p,b)}}
boot();
