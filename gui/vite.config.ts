import {defineConfig} from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import {fileURLToPath, URL} from 'node:url';
export default defineConfig({
 plugins:[react(),tailwindcss()],
 resolve:{alias:{'@':fileURLToPath(new URL('./src',import.meta.url))}},
 base:'./',
 server:{port:18820,strictPort:true},
 build:{outDir:'build',emptyOutDir:true,target:'es2022',sourcemap:true},
 publicDir:'public',
});
