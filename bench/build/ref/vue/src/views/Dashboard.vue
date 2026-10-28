<script setup>
import {ref,computed,onMounted} from 'vue'
import Badge from '../ui/Badge.vue'
import UserForm from '../comp/UserForm.vue'
import api from '../lib/api.js'
import fmt from '../lib/format.js'
import './Dashboard.css'
const users=ref([]),query=ref(''),editing=ref(null),busy=ref(false),error=ref('')
onMounted(()=>api.list().then(v=>users.value=v))
const shown=computed(()=>{const q=query.value.trim().toLowerCase();return q===''?users.value:users.value.filter(u=>u.name.toLowerCase().includes(q)||u.email.toLowerCase().includes(q))})
const activeCount=computed(()=>users.value.filter(u=>u.active).length)
async function save(data){busy.value=true;error.value='';try{const saved=data.id===null?await api.create(data):await api.update(data);users.value=data.id===null?[...users.value,saved]:users.value.map(u=>u.id===saved.id?saved:u);editing.value=null}catch(e){error.value=e.message}busy.value=false}
async function remove(id){error.value='';try{await api.remove(id);users.value=users.value.filter(u=>u.id!==id)}catch(e){error.value=e.message}}
</script>
<template><main class="dash"><header><h1>Users</h1><span>{{activeCount}} active of {{fmt.plural(users.length,'user','users')}}</span>
<input v-model="query" placeholder="Search by name or email" aria-label="Search users"><button @click="editing={}">New</button></header>
<p v-if="error!==''" class="error" role="alert">{{error}}</p>
<UserForm v-if="editing!==null" :key="editing.id??'new'" :user="editing.id?editing:null" :busy="busy" @save="save" @cancel="editing=null" />
<table><thead><tr><th>Name</th><th>Email</th><th>Status</th><th></th></tr></thead><tbody>
<tr v-if="shown.length===0"><td colspan="4">{{query===''?'No users yet.':'No results for this search.'}}</td></tr>
<tr v-for="u in shown" :key="u.id"><td>{{u.name}}</td><td>{{u.email}}</td><td><Badge :tone="u.active?'ok':'bad'" :text="u.active?'Active':'Inactive'" /></td><td><button @click="editing=u">Edit</button><button @click="remove(u.id)">Delete</button></td></tr>
</tbody></table></main></template>
