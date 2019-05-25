import * as path from 'path';
import { Compiler } from './compile';

async function main() {
  const args = process.argv.slice(2);
  const dir = args[0];

  if (!dir) {
    throw new Error('no directory given');
  }

  const rootDir = path.resolve(process.cwd(), dir);
  const c = new Compiler(rootDir);

  await c.addDefaultEntryModule();
}

main();
