// Pure parser facts through the admitted renderer; no files, probes or verdicts.
import {pathToFileURL} from 'node:url';
import {resolve} from 'node:path';
try {
 const chunks=[];for await(const chunk of process.stdin)chunks.push(chunk);
 const {readerPresentation}=await import(pathToFileURL(resolve(process.argv[2],'render.mjs')));
 console.log(JSON.stringify(readerPresentation(JSON.parse(Buffer.concat(chunks).toString('utf8')))));
} catch(error) {
 console.log(JSON.stringify({code:'READER_PRESENTATION_CHECK_FAILED',details:{message:error.message}}));process.exitCode=2;
}
