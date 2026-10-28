<script setup>
import {ref,computed} from 'vue'
import './UserForm.css'
const props=defineProps({user:{default:null},busy:Boolean})
const emit=defineEmits(['save','cancel'])
const name=ref(props.user?.name || ''), email=ref(props.user?.email || ''), active=ref(props.user?.active ?? true)
const valid=computed(()=>name.value.trim()!=='' && email.value.includes('@'))
function save(){if(valid.value&&!props.busy)emit('save',{id:props.user?.id??null,name:name.value.trim(),email:email.value.trim(),active:active.value})}
</script>
<template><form class="user-form" @submit.prevent="save">
<label>Name <input v-model="name"></label><label>Email <input type="email" v-model="email"></label>
<label><input type="checkbox" v-model="active"> Active</label>
<button :disabled="!valid || busy">{{busy?'Saving…':'Save'}}</button><button type="button" @click="emit('cancel')">Cancel</button>
</form></template>
