import link from './linker';
import generate from './generator';

const compile = (source, options) => {
  const ast = link({ source, options });
  return generate({ source, ast, options });
};

export default compile;
