import { tokenize } from './tokenizer';
import { parse } from './parser';
import { transformFromAst } from 'babel-core';

const targetOptions = {
  ES5: {
    presets: ['env']
  },
  default: {
    plugins: []
  }
};

const generate = ({ source, options = {} }) => {
  const tokens = tokenize({ source });
  const ast = parse({ tokens, source });

  const target = targetOptions[options.target] || targetOptions.default;
  // TODO: validate given target

  const transformed = transformFromAst(ast, source, {
    code: true,
    babelrc: false,
    minified: options.minify || false,
    ...target
  });

  return transformed.code;
};

export { generate };
