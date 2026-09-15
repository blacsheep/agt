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
for line in sys.stdin:
 v=json.loads(line)
 with (root/'protocol.jsonl').open('a') as log:log.write(json.dumps(v)+'\n')
 method=v.get('method');p=v.get('params',{});id=v.get('id')
 if method=='initialize':reply(id,{'userAgent':'fixture/1'})
 elif method=='account/read':reply(id,{'account':None,'requiresOpenaiAuth':False})
 elif method=='thread/start':
  thread='thread-'+str(time.time_ns());reply(id,{'thread':{'id':thread},'model':'fixture-effective-model'})
 elif method=='thread/resume':thread=p['threadId'];reply(id,{'thread':{'id':thread},'model':'fixture-effective-model'})
 elif method=='turn/start':
  turn='turn-'+str(time.time_ns());reply(id,{'turn':{'id':turn,'status':'inProgress'}})
  note('turn/started',{'threadId':thread,'turn':{'id':turn}})
  text=p['input'][0]['text']
  if 'HANG' in text:
   (root/'started').write_text(str(__import__('os').getpid()));time.sleep(60)
  elif 'FAIL' in text:note('turn/completed',{'threadId':thread,'turn':{'id':turn,'status':'failed','error':{'message':'fixture failed'}}})
  elif 'EMPTY' in text:note('turn/completed',{'threadId':thread,'turn':{'id':turn,'status':'completed'}})
  elif 'APPROVE' in text:emit({'id':'approval','method':'item/commandExecution/requestApproval','params':{'threadId':thread,'turnId':turn}})
  else:
   note('item/agentMessage/delta',{'threadId':thread,'turnId':turn,'itemId':'comment','delta':'working preview'})
   note('item/completed',{'threadId':thread,'turnId':turn,'item':{'type':'agentMessage','id':'comment','phase':'commentary','text':'not the final reply'}})
   if 'TOOL' in text:
    emit({'id':'tool','method':'item/tool/call','params':{'threadId':thread,'turnId':turn,'tool':'framework_resource_list','arguments':{'kind':'agent'}}})
   else:finish(thread,turn,'fixture final answer')
 elif id=='tool':finish(thread,turn,'tool completed')
 elif method=='turn/interrupt':reply(id,{});note('turn/completed',{'threadId':thread,'turn':{'id':turn,'status':'interrupted'}})
