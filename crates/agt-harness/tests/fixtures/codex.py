#!/usr/bin/env python3
"""Deterministic app-server protocol fixture; never contacts a model."""
import json, sys, pathlib, time
root=pathlib.Path.cwd()
def emit(v):
 print(json.dumps(v),flush=True)
def note(method,params):
 emit({'method':method,'params':params})
def reply(id,result): emit({'id':id,'result':result})
def finish(thread,turn,text):
 note('item/completed',{'threadId':thread,'turnId':turn,'item':{'type':'agentMessage','id':'final','phase':'final_answer','text':text}})
 note('turn/completed',{'threadId':thread,'turn':{'id':turn,'status':'completed'}})
thread='';turn=''; mode=''
with (root/'argv.jsonl').open('a') as log:log.write(json.dumps(sys.argv[1:])+'\n')
for line in sys.stdin:
 v=json.loads(line)
 with (root/'protocol.jsonl').open('a') as log:log.write(json.dumps(v)+'\n')
 method=v.get('method');p=v.get('params',{});id=v.get('id')
 if method=='initialize':
  if (root/'startup-fails').exists():sys.exit(0)
  reply(id,{'userAgent':'fixture/1'})
 elif method=='config/read':reply(id,{'config':{'mcp_servers':{'fetch':{'command':'uvx'},'off':{'command':'x','enabled':False}}},'origins':{}})
 elif method=='account/read':reply(id,{'account':None,'requiresOpenaiAuth':False})
 elif method=='model/list':
  models=[{'id':'fixture-model','model':'fixture-model','displayName':'Fixture model','description':'Local test model','defaultReasoningEffort':'high','supportedReasoningEfforts':[{'reasoningEffort':'high','description':'High'},{'reasoningEffort':'xhigh','description':'Extra high'}],'isDefault':True,'hidden':False}]
  reply(id,{'data':models,'nextCursor':None})
 elif method=='skills/list':reply(id,{'data':[{'cwd':str(root),'skills':[{'name':'fixture-skill','description':'Test skill'}],'errors':[]}]})
 elif method=='mcpServerStatus/list':reply(id,{'data':[{'name':'fixture-mcp','authStatus':'notLoggedIn','tools':{}}],'nextCursor':None})
 elif method=='thread/name/set':
  if (root/'rename-fails').exists():emit({'id':id,'error':{'code':-32000,'message':'fixture rename failure'}})
  else:reply(id,{})
 elif method=='thread/compact/start':
  if (root/'reject-start').exists():
   emit({'id':id,'error':{'code':-32602,'message':'fixture rejected'}});continue
  reply(id,{})
  if p['threadId']=='cancelable-compact':
   thread=p['threadId'];turn='compact-turn'
   note('turn/started',{'threadId':thread,'turn':{'id':turn}})
   continue
  if p['threadId']=='hang-compact':
   (root/'compacting').write_text('1');time.sleep(60)
  time.sleep(0.1)
  note('item/completed',{'threadId':p['threadId'],'item':{'type':'contextCompaction','id':'compact'}})
  note('turn/completed',{'threadId':p['threadId'],'turn':{'id':'compact-turn','status':'completed'}})
 elif method=='thread/start':
  thread='thread-'+str(time.time_ns());reply(id,{'thread':{'id':thread},'model':'fixture-effective-model','reasoningEffort':'high'})
 elif method=='thread/resume':thread=p['threadId'];reply(id,{'thread':{'id':thread},'model':'fixture-effective-model','reasoningEffort':'high'})
 elif method=='turn/start':
  if (root/'reject-start').exists():
   emit({'id':id,'error':{'code':-32602,'message':'fixture rejected'}});continue
  turn='turn-'+str(time.time_ns());reply(id,{'turn':{'id':turn,'status':'inProgress'}})
  note('turn/started',{'threadId':thread,'turn':{'id':turn}})
  text=p['input'][0]['text']
  if 'CANCELABLE' in text:
   (root/'started').write_text('1')
  elif 'HANG' in text:
   (root/'started').write_text(str(__import__('os').getpid()));time.sleep(60)
  elif 'DISCONNECT' in text:
   print('api_key=fixture-secret',file=sys.stderr,flush=True)
   print('fixture transport exited',file=sys.stderr,flush=True)
   sys.exit(7)
  elif 'INTERRUPTED' in text:note('turn/completed',{'threadId':thread,'turn':{'id':turn,'status':'interrupted'}})
  elif 'DELAYED' in text:
   (root/'started').write_text('1');time.sleep(0.4);finish(thread,turn,'completed after expiry')
  elif 'FAIL' in text:note('turn/completed',{'threadId':thread,'turn':{'id':turn,'status':'failed','error':{'message':'fixture failed'}}})
  elif 'EMPTY' in text:note('turn/completed',{'threadId':thread,'turn':{'id':turn,'status':'completed'}})
  elif text.startswith('RELAY_'):
   mode=text
   kind='permissions' if 'PERMISSIONS' in text else ('fileChange' if 'FILE' in text else 'commandExecution')
   req={'id':71 if kind=='permissions' else 'approval','method':'item/'+kind+'/requestApproval','params':{'threadId':thread,'turnId':turn,'itemId':'item-1','permissions':{'network':{'enabled':True}},'availableDecisions':['accept','decline','cancel']}}
   emit(req)
   if 'DUP' in text:emit(req)
   if 'TERMINAL' in text:finish(thread,turn,'ended without approval')
  elif 'APPROVE' in text:emit({'id':'approval','method':'item/commandExecution/requestApproval','params':{'threadId':thread,'turnId':turn}})
  else:
   if 'PROGRESS' in text:
    note('item/started',{'threadId':thread,'item':{'id':'command-1','type':'commandExecution','command':'secret-command','status':'inProgress'}})
    note('item/completed',{'threadId':thread,'item':{'id':'command-1','type':'commandExecution','aggregatedOutput':'private-output','status':'completed'}})
   note('item/agentMessage/delta',{'threadId':thread,'turnId':turn,'itemId':'comment','delta':'working preview'})
   note('item/completed',{'threadId':thread,'turnId':turn,'item':{'type':'agentMessage','id':'comment','phase':'commentary','text':'not the final reply'}})
   if 'TOOL' in text:
    emit({'id':'tool','method':'item/tool/call','params':{'threadId':thread,'turnId':turn,'tool':'framework_resource_list','arguments':{'kind':'agent'}}})
   else:finish(thread,turn,'fixture final answer')
 elif id in ['approval',71] and method is None and 'result' in v:
  note('serverRequest/resolved',{'threadId':thread,'requestId':id})
  finish(thread,turn,'approval returned')
 elif id=='tool':finish(thread,turn,'tool completed')
 elif method=='turn/interrupt':
  reply(id,{})
  if (root/'wrong-turn-interrupt').exists():note('turn/completed',{'threadId':thread,'turn':{'id':'unrelated','status':'completed'}})
  note('turn/completed',{'threadId':thread,'turn':{'id':turn,'status':'interrupted'}})
