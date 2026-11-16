// Pure stdin/stdout check through the admitted renderer. No files or probes.
import {pathToFileURL} from 'node:url';
import {resolve} from 'node:path';
try {
 const chunks=[];for await(const chunk of process.stdin)chunks.push(chunk);
 const original=JSON.parse(Buffer.concat(chunks).toString('utf8'));
 const {checkFragmentAddresses}=await import(pathToFileURL(resolve(process.argv[2],'render.mjs')));
 console.log(JSON.stringify(checkFragmentAddresses(original)));
} catch(error) {
 console.log(JSON.stringify({status:'FAILED',code:'FRAGMENT_ADDRESS_CONTEXT_CHECK_FAILED',details:{message:error.message}}));
 process.exitCode=2;
}
