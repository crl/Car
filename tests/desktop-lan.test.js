import test from 'node:test';
import assert from 'node:assert/strict';
import {existsSync,readFileSync} from 'node:fs';
import vm from 'node:vm';
import {JSDOM} from 'jsdom';
import {createTextMatcher} from '../text-search.js';

const read=name=>readFileSync(new URL('../'+name,import.meta.url),'utf8');
const script=name=>read(name).replace(/^import .*;$/gm,'').replace(/^export /gm,'');
const adminUrl='https://192.168.1.2:8766/';
const guestUrl='http://192.168.1.2:8765/';
const ledger={
 cars:[{id:1,plate:'沪A12345',note:'长途'}],
 drivers:[{id:1,name:'张师傅',phone:'13800138000',note:'夜班',car_ids:[1]}],
 categories:[{id:1,kind:'收入',name:'运费',active:1,requires_route:0},{id:2,kind:'支出',name:'油费',active:1,requires_route:0}],
 addresses:[{id:1,name:'上海仓库',detail:'8号门',active:1}],
 entries:[{id:1,car_id:1,date:'2026-11-26',kind:'收入',category:'运费',cents:12345,start_address_id:null,end_address_id:null,start_address:null,end_address:null,start_address_detail:'',end_address_detail:'',party:'客户',note:''}]
};
const networks=[{id:'Ethernet:192.168.1.2',name:'以太网',address:'192.168.1.2'},{id:'Wi-Fi:192.168.2.8',name:'Wi-Fi',address:'192.168.2.8'}];
const sharing={enabled:false,running:false,network:null,networks,guest_url:null,admin_url:null,error:null,certificate_available:false};

async function start(url,{native=false,authenticated=false}={}){
 const dom=new JSDOM(read('index.html'),{url,runScripts:'dangerously',pretendToBeVisual:true});
 const w=dom.window,d=w.document,context=dom.getInternalVMContext(),calls=[],requests=[],clipboard=[];
 const scheduled=new Map();
 let nextTimer=0;
 w.setInterval=(callback,delay)=>{scheduled.set(++nextTimer,{callback,delay});return nextTimer};
 w.clearInterval=id=>scheduled.delete(id);
 const data=structuredClone(ledger);
 let session=authenticated,lanStatus=structuredClone(sharing),nextLanStatus;
 const nativeResult=async(command,args)=>{
  calls.push([command,args]);
  switch(command){
   case 'startup_status': return {initialized:true};
   case 'account_status': return {authenticated:session};
   case 'read_settings': return {database_path:'ledger.db',backup_path:'backups',default_path:'ledger.db'};
   case 'read_data': return data;
   case 'lan_status': return lanStatus;
   case 'set_lan_sharing':
    lanStatus=nextLanStatus??{...sharing,enabled:args.enabled,running:args.enabled,network:args.network,guest_url:args.enabled?guestUrl:null,admin_url:args.enabled?adminUrl:null,certificate_available:args.enabled};
    nextLanStatus=null;
    return lanStatus;
   case 'export_lan_certificate': return {cancelled:false};
   default: throw Error('命令不可用 '+command);
  }
 };
 const http=async(path,options={})=>{
  const method=options.method??'GET',body=options.body?JSON.parse(options.body):undefined;
  requests.push({path,method,body,credentials:options.credentials});
  const response=(result,status=200)=>new Response(JSON.stringify(result),{status,headers:{'Content-Type':'application/json'}});
  if(path==='/api/account'&&method==='GET')return response({authenticated:session,admin_url:adminUrl});
  if(path==='/api/data'&&method==='GET')return response(data);
  if(path.startsWith('/api/regions?')&&method==='GET')return response({items:[{code:'310000',name:'上海市',path:'上海市',level:1}]});
  if(path==='/api/login'&&method==='POST'&&w.location.protocol==='https:'){
   if(body.username==='root'&&body.password==='s3cret'){session=true;return response({authenticated:true})}
   return response({error:'密码错误'},401);
  }
  if(path==='/api/logout'&&method==='POST'&&w.location.protocol==='https:'){
   if(!session)return response({error:'会话已失效'},401);
   session=false;
   return response({authenticated:false});
  }
  if(method==='POST'&&/^\/api\/(cars|drivers|entries|categories|addresses)(\/delete)?$/.test(path)){
   if(!session)return response({error:'会话已失效'},401);
   const [, ,resource,action]=path.split('/');
   if(action==='delete')data[resource]=data[resource].filter(row=>row.id!==body.id);
   else{
    const record=data[resource].find(row=>row.id===body.id);
    if(record)Object.assign(record,body);
    else data[resource].push({...body,id:2,active:1,car_ids:[]});
   }
   return response(data);
  }
  return response({error:'接口不可用'},404);
 };
 w.isTauri=native;
 w.invoke=nativeResult;
 w.fetch=http;
 Object.defineProperty(w.navigator,'clipboard',{value:{writeText:async value=>clipboard.push(value)}});
 w.getCurrentWindow=()=>({onCloseRequested:async()=>{}});
 w.listen=async()=>{};
 w.getVersion=async()=>'0.1.8';
 w.createUpdater=()=>({checkNow:async()=>{},dispose(){}});
 w.createTextMatcher=createTextMatcher;
 w.HTMLDialogElement.prototype.showModal=function(){this.open=true};
 w.HTMLDialogElement.prototype.close=function(){this.open=false;this.dispatchEvent(new w.Event('close'))};
 const matches=w.Element.prototype.matches;
 w.Element.prototype.matches=function(selector){return selector===':popover-open'?this.hasAttribute('data-open'):matches.call(this,selector)};
 w.HTMLElement.prototype.showPopover=function(){this.setAttribute('data-open','')};
 w.HTMLElement.prototype.hidePopover=function(){this.removeAttribute('data-open')};
 const fileSource=name=>existsSync(new URL('../'+name,import.meta.url))?script(name):'';
 w.loadApp=()=>vm.runInContext('(async()=>{'+script('app.js')+'})()',context);
 const entry=script('desktop.js').replaceAll("import('./app.js')",'loadApp()').replace(/boot\(\);\s*$/,'');
 await vm.runInContext(fileSource('web-service.js')+'\n'+fileSource('lan-settings.js')+'\n'+entry+'\nglobalThis.api=service;boot()',context);
 return {w,d,requests,calls,clipboard,data,
  expire(){session=false},
  lanResult(status){nextLanStatus=status},
  monitorResult(status){lanStatus=status},
  async elapseLanInterval(){await Promise.all([...scheduled.values()].filter(timer=>timer.delay===5000).map(timer=>timer.callback()))},
  close:()=>w.close()
 };
}
const tick=()=>new Promise(resolve=>setTimeout(resolve,0));

test('HTTP guests open the requested vehicle and can follow an HTTPS link for the current workspace',async()=>{
 const h=await start(guestUrl+'#/cars/1',{authenticated:true}),{w,d}=h;
 try{
  assert.equal(d.querySelector('#pageTitle').textContent,'沪A12345');
  assert.match(d.querySelector('#vehicleEntries').textContent,/运费/);
  assert.equal(d.querySelector('#webAdminLink').href,adminUrl+'#/cars/1');
  assert.equal(d.querySelector('#webAdminLink').hidden,false);
  assert.equal(d.querySelector('#editVehicleProfile'),null);
  assert.equal(d.querySelector('.settings-trigger'),null);
  assert.equal(d.querySelector('[onclick="exportVehicleRows()"]'),null);
  w.location.hash='#/drivers';
  await new Promise(resolve=>w.addEventListener('hashchange',resolve,{once:true}));
  assert.equal(d.querySelector('#webAdminLink').href,adminUrl+'#/drivers');
  d.querySelector('#driverSearch').value='1380013';
  d.querySelector('#driverSearch').dispatchEvent(new w.Event('input'));
  assert.match(d.querySelector('#driverRows').textContent,/张师傅.*13800138000/);
  assert.deepEqual(h.calls,[]);
  assert.equal(h.requests.some(request=>request.method==='POST'),false);
 }finally{h.close()}
});

test('HTTPS administrators use the shared login form and save a driver through the web API',async()=>{
 const h=await start(adminUrl+'#/drivers'),{w,d,requests}=h;
 try{
  d.querySelector('#accountButton').click();
  const login=d.querySelector('#loginForm');
  assert.ok(login,'HTTPS serves the existing login form');
  login.elements.password.value='incorrect';
  await login.onsubmit({preventDefault(){},currentTarget:login});
  assert.match(d.querySelector('#accountMessage').textContent,/密码错误/);
  login.elements.password.value='s3cret';
  await login.onsubmit({preventDefault(){},currentTarget:login});
  assert.equal(d.querySelector('#accountName').textContent,'管理员');
  d.querySelector('#driverRows button').click();
  const form=d.querySelector('#form');
  form.elements.phone.value='13900139000';
  await form.onsubmit({preventDefault(){}});
  assert.match(d.querySelector('#driverRows').textContent,/13900139000/);
  assert.equal(requests.find(request=>request.path==='/api/drivers').method,'POST');
  assert.equal(requests.find(request=>request.path==='/api/drivers').credentials,'same-origin');
  w.navigateWorkspace('#/entries');
  assert.ok(d.querySelector('[onclick="entryForm()"]'));
  assert.equal(d.querySelector('[onclick="batchImport()"]'),null);
  d.querySelector('#accountButton').click();
  assert.equal(d.querySelector('#passwordForm'),null);
  await w.accountLogout();
  assert.equal(d.querySelector('#accountName').textContent,'游客');
  assert.equal(d.querySelector('[onclick="entryForm()"]'),null);
  assert.deepEqual(h.calls,[]);
 }finally{h.close()}
});

test('HTTPS session expiry prompts login and keeps the unsaved record available for retry',async()=>{
 const h=await start(adminUrl+'#/cars/1',{authenticated:true}),{d}=h;
 try{
  d.querySelector('#editVehicleProfile').click();
  const form=d.querySelector('#form');
  form.elements.note.value='尚未提交的备注';
  h.expire();
  await form.onsubmit({preventDefault(){}});
  assert.equal(d.querySelector('#accountModal').open,true);
  assert.match(d.querySelector('#accountMessage').textContent,/重新登录/);
  assert.equal(d.querySelector('#modal').open,true);
  assert.equal(form.elements.note.value,'尚未提交的备注');
  const login=d.querySelector('#loginForm');
  login.elements.password.value='s3cret';
  await login.onsubmit({preventDefault(){},currentTarget:login});
  assert.equal(d.querySelector('#modal').open,true);
  assert.equal(form.elements.note.value,'尚未提交的备注');
  await form.onsubmit({preventDefault(){}});
  assert.match(d.querySelector('.vehicle-profile').textContent,/尚未提交的备注/);
  assert.equal(d.querySelector('#modal').open,false);
 }finally{h.close()}
});

test('logging out an expired HTTPS session returns the workspace to guest mode',async()=>{
 const h=await start(adminUrl+'#/cars/1',{authenticated:true}),{w,d}=h;
 try{
  d.querySelector('#accountButton').click();
  h.expire();
  await w.accountLogout();
  assert.equal(d.querySelector('#accountModal').open,false);
  assert.equal(d.querySelector('#accountName').textContent,'游客');
  assert.equal(d.querySelector('#editVehicleProfile'),null);
 }finally{h.close()}
});

test('desktop network choice starts sharing and exposes copy, certificate export and stop',async()=>{
 const h=await start('http://tauri.localhost/#/cars',{native:true,authenticated:true}),{w,d}=h;
 try{
  await w.openSiteSettings();
  const section=d.querySelector('#lanSettings'),select=section.querySelector('select');
  assert.match(section.textContent,/局域网共享/);
  assert.doesNotMatch(section.textContent,/未开启/);
  assert.equal(select.options.length,networks.length);
  const toggle=section.querySelector('#lanToggle');
  assert.equal(toggle?.textContent,'开启共享');
  select.value=networks[0].id;
  toggle.click();
  await tick();
  assert.doesNotMatch(section.textContent,/运行中/);
  assert.equal(toggle.textContent,'停止共享');
  assert.equal(section.querySelector('#lanRetry').hidden,true);
  assert.equal(section.querySelector('#lanGuestUrl').value,guestUrl);
  assert.equal(section.querySelector('#lanAdminUrl').value,adminUrl);
  section.querySelector('#lanCopyGuest').click();
  section.querySelector('#lanCopyAdmin').click();
  await tick();
  assert.deepEqual(h.clipboard,[guestUrl,adminUrl]);
  section.querySelector('#lanExportCertificate').click();
  await tick();
  assert.equal(h.calls.at(-1)[0],'export_lan_certificate');
  toggle.click();
  await tick();
  assert.equal(toggle.textContent,'开启共享');
  assert.equal(section.querySelector('#lanGuestUrl').value,'');
 }finally{h.close()}
});

test('desktop sharing surfaces the listener error returned by the host',async()=>{
 const h=await start('http://tauri.localhost/#/cars',{native:true,authenticated:true}),{w,d}=h;
 try{
  await w.openSiteSettings();
  h.lanResult({...sharing,enabled:true,network:networks[0].id,error:'端口已被占用'});
  const toggle=d.querySelector('#lanToggle');
  toggle?.click();
  await tick();
  assert.match(d.querySelector('#lanSettings').textContent,/端口已被占用/);
  assert.equal(toggle.textContent,'停止共享');
  const retry=d.querySelector('#lanRetry');
  assert.equal(retry.hidden,false);
  retry.click();
  await tick();
  assert.equal(d.querySelector('#lanGuestRow').hidden,false);
  assert.equal(retry.hidden,true);
 }finally{h.close()}
});

test('open desktop settings refreshes LAN status without resetting the selected adapter',async()=>{
 const h=await start('http://tauri.localhost/#/cars',{native:true,authenticated:true}),{w,d}=h;
 try{
  await w.openSiteSettings();
  const select=d.querySelector('#lanNetwork');
  select.value=networks[1].id;
  select.dispatchEvent(new w.Event('change'));
  h.monitorResult({...sharing,enabled:true,network:networks[0].id,error:'网络连接已断开'});
  await h.elapseLanInterval();
  assert.match(d.querySelector('#lanError').textContent,/网络连接已断开/);
  assert.equal(select.value,networks[1].id);
  w.closeSiteSettings();
  const calls=h.calls.length;
  await h.elapseLanInterval();
  assert.equal(h.calls.length,calls);
 }finally{h.close()}
});
