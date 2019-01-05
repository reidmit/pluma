import { tokenize } from './tokenize';
import { parse } from './parse';

const input = `
# a comment
x = 47
print x {
  . ,  = = [ 29 lol nice ]
}
`.repeat(1);

console.time('tokenize');
const tokens = tokenize(input);
console.timeEnd('tokenize');

// console.time('parse');
// const ast = parse(tokens);
// console.timeEnd('parse');

console.log();
console.log(tokens);
console.log();
// console.log(ast);
