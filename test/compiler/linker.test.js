import link from '../../src/compiler/linker';
import path from 'path';

const sourceDirectory = path.resolve(__dirname, './fixtures');

describe('linker', () => {
  test('a single file with no imports', () => {
    const { asts } = link({
      entry: path.resolve(sourceDirectory, 'Main.plum')
    });

    expect(asts).toHaveLength(1);
    expect(asts[0].moduleName).toBe('Main');
  });

  test('a single file with exports', () => {
    const { asts, entryExports } = link({
      entry: path.resolve(sourceDirectory, 'Exporting.plum')
    });

    expect(asts).toHaveLength(1);
    expect(asts[0].moduleName).toBe('Exporting');
    expect(entryExports).toEqual('module$Exporting');
  });

  test('multiple files with imports and no export from entry point', () => {
    const { asts, entryExports } = link({
      entry: path.resolve(sourceDirectory, 'Importing.plum')
    });

    expect(asts).toHaveLength(3);
    expect(asts[0].moduleName).toBe('Subdirectory.Another');
    expect(asts[1].moduleName).toBe('Main');
    expect(asts[2].moduleName).toBe('Importing');
    expect(entryExports).toEqual(null);
  });

  test('fails helpfully on circular dependencies', () => {
    let error;

    try {
      link({
        entry: path.resolve(sourceDirectory, 'Circular/CircularA.plum')
      });
    } catch (err) {
      error = err;
    }

    expect(error).toBeDefined();
    expect(error.message).toContain('CircularA --> CircularB --> CircularA');
    expect(error.message).toContain('CircularB --> CircularA --> CircularB');
  });
});
