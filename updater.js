// UI state is independent of Tauri so download, retry and unsaved-work behavior
// can be exercised without installing anything on the test machine.
export function createUpdater({document,check,prepareInstall,restart,releaseInstall}){
 const brand=document.querySelector('.brand');
 const icon=brand.querySelector('.brand-icon');
 const wrap=document.createElement('span');wrap.className='brand-icon-wrap';
 icon.before(wrap);wrap.append(icon);
 const dot=document.createElement('span');dot.className='update-dot';dot.hidden=true;
 dot.setAttribute('aria-hidden','true');wrap.append(dot);
 const dialog=document.createElement('dialog');dialog.id='updateDialog';
 dialog.setAttribute('aria-labelledby','updateTitle');
 dialog.innerHTML='<div class="modal-head"><h2 id="updateTitle">软件更新</h2><button type="button" class="icon-button" aria-label="关闭更新窗口">×</button></div><div class="modal-body"><p id="updateStatus" role="status" aria-live="polite"></p><pre id="updateNotes"></pre><progress id="updateProgress" max="100" hidden></progress></div><div class="modal-footer row"><button type="button" class="secondary" id="updateCheck">检查更新</button><button type="button" id="updateAction" hidden>下载并更新</button></div>';
 document.body.append(dialog);
 const status=dialog.querySelector('#updateStatus'),notes=dialog.querySelector('#updateNotes');
 const progress=dialog.querySelector('#updateProgress'),action=dialog.querySelector('#updateAction');
 const retry=dialog.querySelector('#updateCheck'),close=dialog.querySelector('.icon-button');
 let update=null,busy=false,checking=false,downloaded=false,disposed=false;
 function refresh(){
  dot.hidden=!update;
  brand.setAttribute('aria-label',update?`运管家，有新版本 v${update.version}，查看更新`:'运管家，返回车辆首页');
  brand.title=update?`发现新版本 v${update.version}，点击更新`:'';
  action.hidden=!update;action.disabled=busy||checking;retry.disabled=busy||checking;close.disabled=busy;
 }
 function open(){if(!dialog.open)dialog.showModal()}
 function brandClick(e){if(update){e.preventDefault();open()}}
 brand.addEventListener('click',brandClick);
 close.onclick=()=>dialog.close();retry.onclick=()=>checkNow();action.onclick=()=>startUpdate();
 dialog.addEventListener('cancel',e=>{if(busy)e.preventDefault()});
 async function checkNow(){
  if(checking||busy||disposed)return;
  checking=true;status.textContent='正在检查更新…';refresh();
  try{
   const next=await check({timeout:15000});
   if(disposed){await next?.close();return}
   await update?.close().catch(()=>{});update=next;downloaded=false;
   notes.textContent=update?.body||'';
   status.textContent=update?`发现新版本 v${update.version}`:'当前已是最新版本';
   progress.hidden=true;
  }catch{status.textContent='检查更新失败，请检查网络后重试。'}
  finally{checking=false;if(!disposed)refresh()}
 }
 async function startUpdate(){
  if(!update||busy||checking||disposed)return;
  busy=true;open();refresh();let prepared=false,installed=false;
  try{
   if(!downloaded){
    let received=0,total=0;progress.hidden=false;progress.removeAttribute('value');
    status.textContent='正在下载更新…';
    await update.download(event=>{
     if(event.event==='Started')total=event.data.contentLength||0;
     if(event.event==='Progress')received+=event.data.chunkLength;
     if(total){const percent=Math.min(100,Math.floor(received/total*100));progress.value=percent;status.textContent=`正在下载更新… ${percent}%`}
    },{timeout:120000});
    downloaded=true;progress.value=100;
   }
   if(!await prepareInstall()){status.textContent='已下载更新。处理完未保存内容或账本操作后，可再次点击更新。';return}
   prepared=true;status.textContent='正在安装更新，请稍候…';
   await update.install({restartAfterInstall:true});installed=true;
   status.textContent='更新已安装，正在重启…';
   await restart();
  }catch{
   downloaded=false;
   status.textContent=installed?'更新已安装，自动重启失败，请关闭并重新打开运管家。':'更新失败，当前账本未受影响。请检查网络或安装权限后重试。';
  }finally{
   if(prepared)await releaseInstall().catch(()=>{});
   busy=false;refresh();
   if(installed){update=null;refresh();action.hidden=true}
  }
 }
 refresh();
 return {checkNow,startUpdate,open,get busy(){return busy},dispose(){disposed=true;brand.removeEventListener('click',brandClick);void update?.close().catch(()=>{});dialog.remove();wrap.replaceWith(icon)}};
}
