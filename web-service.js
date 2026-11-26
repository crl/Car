const recordWrite=/^\/api\/(cars|drivers|entries|categories|addresses)(\/delete)?$/;

export function createWebService(send,{secure,onSessionExpired}){
 return {
  async request(path,options={}){
   const method=options.method??'GET';
   if(method==='GET'&&(path==='/api/data'||path.startsWith('/api/regions?'))){
    return send(path,{credentials:'same-origin'});
   }
   if(method!=='POST'||!recordWrite.test(path)||!secure)throw Error('浏览器不支持此操作');
   const response=await send(path,{...options,credentials:'same-origin'});
   if(response.status===401)onSessionExpired();
   return response;
  },
  async call(command,args={}){
   if(command==='account_status'){
    const response=await send('/api/account',{credentials:'same-origin'});
    return readAccountResponse(response);
   }
   if(!secure||!['login','logout'].includes(command))throw Error('浏览器不支持此操作');
   const response=await send('/api/'+command,{
    method:'POST',credentials:'same-origin',headers:{'Content-Type':'application/json'},body:JSON.stringify(args)
   });
   if(command==='logout'&&response.status===401)return {authenticated:false};
   return readAccountResponse(response);
  }
 };
}

async function readAccountResponse(response){
 const result=await response.json();
 if(!response.ok)throw Error(result.error||'账号操作失败，请重试');
 return result;
}
