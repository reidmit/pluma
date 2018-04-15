import link from './linker';
import generate from './generator';

function compile(source, options) {
  const { asts, entryExports } = link({ source, options });
  return generate({ source, asts, entryExports, options });
}

export default compile;
