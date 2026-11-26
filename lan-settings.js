export function createLanSettings(container,service){
 const section=document.createElement('section');
 section.id='lanSettings';
 section.innerHTML=`
  <h2>局域网共享</h2>
  <p class="help">同一局域网内的设备可通过浏览器查看账本，管理员可通过 HTTPS 登录编辑。</p>
  <p id="lanStatus" role="status"></p>
  <p id="lanError" role="alert" hidden></p>
  <div id="lanControls" hidden>
   <label>共享网络<select id="lanNetwork"></select></label>
   <div class="lan-actions">
    <button id="lanStart" type="button">开启共享</button>
    <button id="lanStop" type="button" class="secondary">停止共享</button>
   </div>
  </div>
  <div id="lanGuestRow" hidden>
   <label>访客地址<input id="lanGuestUrl" readonly></label>
   <button id="lanCopyGuest" type="button" class="secondary">复制访客地址</button>
  </div>
  <div id="lanAdminRow" hidden>
   <label>管理员地址<input id="lanAdminUrl" readonly></label>
   <button id="lanCopyAdmin" type="button" class="secondary">复制管理员地址</button>
  </div>
  <div id="lanCertificate" hidden>
   <p class="help">首次从其他设备登录前，将本机导出的 CA 证书（.cer）通过可信渠道发送到该设备，并加入系统信任，然后打开管理员地址。</p>
   <button id="lanExportCertificate" type="button" class="secondary">导出 CA 证书</button>
  </div>
  <p id="lanFeedback" class="help" role="status"></p>`;
 container.prepend(section);
 const find=id=>section.querySelector('#'+id);
 const select=find('lanNetwork'),start=find('lanStart'),stop=find('lanStop'),certificate=find('lanExportCertificate'),feedback=find('lanFeedback');
 let state=null,canManage=false,busy=false,reading=false,sharingRevision=0;
 let selectedNetwork=null,networkOptions=[];
 select.onchange=()=>{selectedNetwork=select.value};

 function render(){
  find('lanControls').hidden=!canManage;
  find('lanStatus').textContent=!state?'正在读取共享状态…':state.running?'运行中':state.enabled?'已启用，服务未运行':'未开启';
  const error=find('lanError');
  error.textContent=state?.error??'';
  error.hidden=!error.textContent;
  const networks=state?.networks??[];
  const selected=state?.running?state.network:selectedNetwork??state?.network;
  if(!select.options.length||networks.length!==networkOptions.length||networks.some((network,i)=>{
   const option=networkOptions[i];
   return network.id!==option.id||network.name!==option.name||network.address!==option.address;
  })){
   select.replaceChildren(...networks.map(network=>new Option(`${network.name} · ${network.address}`,network.id)));
   if(!networks.length)select.add(new Option('未检测到可用网络',''));
   networkOptions=networks;
  }
  if(networks.some(network=>network.id===selected)&&select.value!==selected)select.value=selected;
  select.disabled=busy||!networks.length||Boolean(state?.running);
  start.disabled=busy||!select.value||Boolean(state?.running);
  stop.disabled=busy||!state?.enabled;
  find('lanCertificate').hidden=!canManage||!state?.certificate_available;
  certificate.disabled=busy;
  find('lanGuestRow').hidden=!state?.guest_url;
  find('lanGuestUrl').value=state?.guest_url??'';
  find('lanAdminRow').hidden=!state?.admin_url;
  find('lanAdminUrl').value=state?.admin_url??'';
 }

 async function updateSharing(enabled){
  if(busy||!canManage)return;
  const network=enabled?select.value:state.network;
  sharingRevision++;
  busy=true;
  feedback.textContent=enabled?'正在启动共享…':'正在停止共享…';
  render();
  try{
   state=await service.call('set_lan_sharing',{enabled,network});
   if(state.running)selectedNetwork=null;
   feedback.textContent='';
  }catch(error){
   feedback.textContent=String(error);
  }finally{
   busy=false;
   render();
  }
 }

 function connectCopy(button,input){
  find(button).onclick=async()=>{
   const field=find(input);
   try{
    await navigator.clipboard.writeText(field.value);
    feedback.textContent='地址已复制';
   }catch{
    field.focus();
    field.select();
    feedback.textContent='已选中地址，请手动复制。';
   }
  };
 }
 connectCopy('lanCopyGuest','lanGuestUrl');
 connectCopy('lanCopyAdmin','lanAdminUrl');
 start.onclick=()=>updateSharing(true);
 stop.onclick=()=>updateSharing(false);
 certificate.onclick=async()=>{
  if(busy||!canManage)return;
  busy=true;
  feedback.textContent='';
  render();
  try{
   const result=await service.call('export_lan_certificate');
   if(!result.cancelled)feedback.textContent='CA 证书已导出';
  }catch(error){
   feedback.textContent=String(error);
  }finally{
   busy=false;
   render();
  }
 };

 return async (admin,{quiet=false}={})=>{
  canManage=admin;
  if(busy||reading)return;
  if(!quiet){
   feedback.textContent='';
   render();
  }
  reading=true;
  const revision=sharingRevision;
  try{
   const next=await service.call('lan_status');
   if(revision===sharingRevision){
    state=next;
    render();
   }
  }catch(error){
   if(!quiet&&revision===sharingRevision){
    feedback.textContent='读取共享状态失败：'+String(error);
    if(!state)find('lanStatus').textContent='共享状态不可用';
   }
  }finally{
   reading=false;
  }
 };
}
