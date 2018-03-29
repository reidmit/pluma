import { formatSourceBlock } from './error-helper';

const source = `let a = 10
let b = 20
let b0 = 20
let b1 = 20
let b2 = 20
let b3 = 20
let c = 30
let d = 40
let error = 50
let f = 60
let g = 70
let h = 80
let i = 90
`;

describe('formatSourceBlock', () => {
  test('defaults', () => {
    const lineNumber = 9;
    const columnStart = 4;

    const sourceBlock = formatSourceBlock({
      source,
      lineNumber,
      columnStart,
      useColor: false
    });

    expect(sourceBlock).toBe(
      [
        '   7 | let c = 30',
        '   8 | let d = 40',
        ' > 9 | let error = 50',
        '           ^',
        '  10 | let f = 60',
        '  11 | let g = 70'
      ].join('\n')
    );
  });

  test('with a columnEnd', () => {
    const lineNumber = 9;
    const columnStart = 4;
    const columnEnd = 9;

    const sourceBlock = formatSourceBlock({
      source,
      lineNumber,
      columnStart,
      columnEnd,
      useColor: false
    });

    expect(sourceBlock).toBe(
      [
        '   7 | let c = 30',
        '   8 | let d = 40',
        ' > 9 | let error = 50',
        '           ^^^^^',
        '  10 | let f = 60',
        '  11 | let g = 70'
      ].join('\n')
    );
  });

  test('with surroundingLines = 0', () => {
    const lineNumber = 9;
    const columnStart = 4;
    const columnEnd = 9;

    const sourceBlock = formatSourceBlock({
      source,
      lineNumber,
      columnStart,
      columnEnd,
      surroundingLines: 0,
      useColor: false
    });

    expect(sourceBlock).toBe(
      [' > 9 | let error = 50', '           ^^^^^'].join('\n')
    );
  });
});
