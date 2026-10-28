import {Component,Input,Output,EventEmitter,signal,computed} from '@angular/core';
import {bootstrapApplication} from '@angular/platform-browser';
import {FormsModule} from '@angular/forms';
import api from './api.js';
@Component({selector:'bench-badge',standalone:true,template:'<span class="badge" [attr.data-tone]="tone">{{text}}</span>'})
class Badge { @Input() tone='neutral'; @Input() text=''; }
@Component({selector:'bench-user-form',standalone:true,imports:[FormsModule],template:`
<form class="user-form" (ngSubmit)="submit()">
<label>Name <input name="name" [(ngModel)]="name"></label>
<label>Email <input name="email" type="email" [(ngModel)]="email"></label>
<label><input name="active" type="checkbox" [(ngModel)]="active"> Active</label>
<button [disabled]="!valid || busy">{{busy?'Saving…':'Save'}}</button><button type="button" (click)="cancel.emit()">Cancel</button>
</form>`})
class UserForm {
 private current:any=null; name=''; email=''; active=true;
 @Input() set user(v:any){this.current=v;this.name=v?.name||'';this.email=v?.email||'';this.active=v?.active??true;}
 @Input() busy=false;
 @Output() save=new EventEmitter<any>(); @Output() cancel=new EventEmitter<void>();
 get valid(){return this.name.trim()!==''&&this.email.includes('@');}
 submit(){if(this.valid&&!this.busy)this.save.emit({id:this.current?.id??null,name:this.name.trim(),email:this.email.trim(),active:this.active});}
}
@Component({selector:'bench-dashboard',standalone:true,imports:[FormsModule,Badge,UserForm],template:`
<main class="dash"><header><h1>Users</h1><span>{{activeCount()}} active of {{users().length}} {{users().length===1?'user':'users'}}</span>
<input [ngModel]="query()" (ngModelChange)="query.set($event)" placeholder="Search by name or email" aria-label="Search users"><button (click)="editing.set({})">New</button></header>
@if(error()!==''){<p class="error" role="alert">{{error()}}</p>}
@if(editing()!==null){<bench-user-form [user]="editing()?.id?editing():null" [busy]="busy()" (save)="save($event)" (cancel)="editing.set(null)" />}
<table><thead><tr><th>Name</th><th>Email</th><th>Status</th><th></th></tr></thead><tbody>
@if(shown().length===0){<tr><td colspan="4">{{query()===''?'No users yet.':'No results for this search.'}}</td></tr>}
@for(u of shown();track u.id){<tr><td>{{u.name}}</td><td>{{u.email}}</td><td><bench-badge [tone]="u.active?'ok':'bad'" [text]="u.active?'Active':'Inactive'" /></td><td><button (click)="editing.set(u)">Edit</button><button (click)="remove(u.id)">Delete</button></td></tr>}
</tbody></table></main>`})
class Dashboard {
 users=signal<any[]>([]);query=signal('');editing=signal<any>(null);busy=signal(false);error=signal('');
 shown=computed(()=>{const q=this.query().trim().toLowerCase();return q===''?this.users():this.users().filter(u=>u.name.toLowerCase().includes(q)||u.email.toLowerCase().includes(q));});
 activeCount=computed(()=>this.users().filter(u=>u.active).length);
 constructor(){api.list().then(v=>this.users.set(v));}
 async save(data:any){this.busy.set(true);this.error.set('');try{const saved=data.id===null?await api.create(data):await api.update(data);this.users.update(prev=>data.id===null?[...prev,saved]:prev.map(u=>u.id===saved.id?saved:u));this.editing.set(null);}catch(e:any){this.error.set(e.message);}this.busy.set(false);}
 async remove(id:number){this.error.set('');try{await api.remove(id);this.users.update(prev=>prev.filter(u=>u.id!==id));}catch(e:any){this.error.set(e.message);}}
}
bootstrapApplication(Dashboard).catch(e=>console.error(e));
