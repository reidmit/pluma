import { tokenize } from './tokenizer';
import { parse } from './parser';
import { generate } from './generator';

const compile = (source, options) => {
  const tokens = tokenize({ source, options });
  const ast = parse({ source, tokens, options });
  return generate({ source, tokens, ast, options });
};

export { compile };
