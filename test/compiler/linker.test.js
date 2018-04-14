import link from '../../src/compiler/linker';
import path from 'path';

const sourceDirectory = path.resolve(__dirname, './fixtures');

describe('linker', () => {
  test('a single file with no imports', () => {
    const linkedAsts = link({
      entry: path.resolve(sourceDirectory, 'Main.plum')
    });

    expect(linkedAsts).toHaveLength(1);
    expect(linkedAsts[0].moduleName).toBe('Main');
  });

  test('multiple files with imports', () => {
    const linkedAsts = link({
      entry: path.resolve(sourceDirectory, 'Importing.plum')
    });

    expect(linkedAsts).toHaveLength(3);
    expect(linkedAsts[0].moduleName).toBe('Subdirectory.Another');
    expect(linkedAsts[1].moduleName).toBe('Main');
    expect(linkedAsts[2].moduleName).toBe('Importing');
  });

  test.only('fails helpfully on circular dependencies', () => {
    const linkedAsts = link({
      entry: path.resolve(sourceDirectory, 'Circular/CircularA.plum')
    });
  });
});
