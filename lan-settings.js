export function createLanSettings(container,service){
 const section=document.createElement('section');
 section.id='lanSettings';
 section.innerHTML=`
  <h2>局域网共享</h2>
  <div id="lanControls" class="lan-setting-row" hidden>
   <label for="lanNetwork">共享网络</label>
   <select id="lanNetwork"></select>
   <button id="lanToggle" type="button"></button>
  </div>
  <div id="lanProblem" class="lan-inline-row" hidden>
   <p id="lanError" role="alert" hidden></p>
   <button id="lanRetry" type="button" class="secondary" hidden>重试</button>
  </div>
  <div id="lanGuestRow" class="lan-setting-row" hidden>
   <label for="lanGuestUrl">访客地址</label>
   <input id="lanGuestUrl" readonly>
   <button id="lanCopyGuest" type="button" class="secondary" aria-label="复制访客地址">复制</button>
  </div>
  <div id="lanAdminRow" class="lan-setting-row" hidden>
   <label for="lanAdminUrl">管理员地址</label>
   <input id="lanAdminUrl" readonly>
   <button id="lanCopyAdmin" type="button" class="secondary" aria-label="复制管理员地址">复制</button>
  </div>
  <div id="lanCertificate" class="lan-inline-row" hidden>
   <p class="help">首次管理前，请通过可信渠道将证书（.cer）传到访问设备并设为受信任。</p>
   <button id="lanExportCertificate" type="button" class="secondary" aria-label="导出 CA 证书">导出证书</button>
  </div>
  <p id="lanFeedback" class="help" role="status"></p>`;
 container.prepend(section);
 const find=id=>section.querySelector('#'+id);
 const select=find('lanNetwork'),toggle=find('lanToggle'),retry=find('lanRetry');
 const error=find('lanError'),certificate=find('lanExportCertificate'),feedback=find('lanFeedback');
 let state=null,canManage=false,busy=false,reading=false,sharingRevision=0;
 let selectedNetwork=null,networkOptions=[];
 select.onchange=()=>{selectedNetwork=select.value};

 function render(){
  find('lanControls').hidden=!canManage;
  const enabled=Boolean(state?.enabled),running=Boolean(state?.running);
  error.textContent=state?.error??'';
  error.hidden=!error.textContent;
  const networks=state?.networks??[];
  const selected=running?state.network:selectedNetwork??state?.network;
  if(!select.options.length||networks.length!==networkOptions.length||networks.some((network,i)=>{
   const option=networkOptions[i];
   return network.id!==option.id||network.name!==option.name||network.address!==option.address;
  })){
   select.replaceChildren(...networks.map(network=>new Option(`${network.name} · ${network.address}`,network.id)));
   if(!networks.length)select.add(new Option('未检测到可用网络',''));
   networkOptions=networks;
  }
  if(networks.some(network=>network.id===selected)&&select.value!==selected)select.value=selected;
  select.disabled=busy||!networks.length||running;
  toggle.textContent=enabled?'停止共享':'开启共享';
  toggle.classList.toggle('danger',enabled);
  toggle.disabled=busy||(!enabled&&!select.value);
  retry.hidden=!(canManage&&enabled&&!running&&select.value);
  retry.disabled=busy;
  find('lanProblem').hidden=error.hidden&&retry.hidden;
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
 toggle.onclick=()=>updateSharing(!state?.enabled);
 retry.onclick=()=>updateSharing(true);
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
   }
  }finally{
   reading=false;
  }
 };
}
