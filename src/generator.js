import { tokenize } from './tokenizer';
import { parse } from './parser';
import babelGenerate from 'babel-generator';

const generate = ({ source }) => {
  const tokens = tokenize({ source });
  const ast = parse({ tokens, source });
  const generated = babelGenerate(ast);

  console.log({ generated });
};

export { generate };
