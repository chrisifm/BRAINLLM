// Browser-native decoding: compare pixels, not compressed PNG bytes. Inputs are bounded by the Rust caller.
async (baseline, current, allowed) => {
  const decode = src => new Promise((resolve, reject) => {
    const img = new Image();
    const timer = setTimeout(() => { img.src = ''; reject(new Error('PNG decode exceeded 5 seconds')); }, 5000);
    img.onload = () => { clearTimeout(timer); resolve(img); };
    img.onerror = () => { clearTimeout(timer); reject(new Error('Invalid baseline PNG; create it with brain shot --update-baseline')); };
    img.src = 'data:image/png;base64,' + src;
  });
  const [a,b] = await Promise.all([decode(baseline),decode(current)]);
  if (a.width !== b.width || a.height !== b.height) return {ok:false, reason:'Baseline dimensions differ', changedPixels:null, ratio:1};
  if (a.width * a.height > 4194304) throw new Error('Image exceeds the 4 megapixel comparison limit');
  const canvas = document.createElement('canvas'); canvas.width=a.width; canvas.height=a.height;
  const ctx=canvas.getContext('2d',{willReadFrequently:true});
  ctx.drawImage(a,0,0); const before=ctx.getImageData(0,0,a.width,a.height);
  ctx.clearRect(0,0,a.width,a.height); ctx.drawImage(b,0,0); const after=ctx.getImageData(0,0,b.width,b.height);
  let changed=0;
  for(let i=0;i<before.data.length;i+=4){
    if(before.data[i]!==after.data[i] || before.data[i+1]!==after.data[i+1] || before.data[i+2]!==after.data[i+2] || before.data[i+3]!==after.data[i+3]){
      changed++; after.data[i]=255; after.data[i+1]=0; after.data[i+2]=96; after.data[i+3]=255;
    } else { after.data[i+3]=80; }
  }
  const ratio=changed/(a.width*a.height);
  ctx.putImageData(after,0,0);
  return {ok:ratio<=allowed,changedPixels:changed,totalPixels:a.width*a.height,ratio,diff:changed?canvas.toDataURL('image/png').split(',')[1]:null};
}
