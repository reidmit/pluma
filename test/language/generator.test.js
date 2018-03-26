import generate from '../../src/language/generator';

describe('generate', () => {
  describe('basic examples', () => {
    const source = `
let fn = a => b => c => "hello, world!"

let greet = name => "hi \${name}";

fn 1 2 3

let obj = { a: 1, b
}`;

    test('with default options', () => {
      const compiled = generate({ source });
      expect(compiled).toBe(`const fn = a => b => c => "hello, world!";

const greet = name => \`hi \${name}\`;

fn(1)(2)(3);
const obj = {
  a: 1,
  b
};`);
    });

    test('targeting ES5', () => {
      const compiled = generate({
        source,
        options: {
          target: 'ES5'
        }
      });

      expect(compiled).toBe(`"use strict";

var fn = function fn(a) {
  return function (b) {
    return function (c) {
      return "hello, world!";
    };
  };
};

var greet = function greet(name) {
  return "hi " + name;
};

fn(1)(2)(3);
var obj = {
  a: 1,
  b: b
};`);
    });

    test('targeting ES5 & minifying', () => {
      const compiled = generate({
        source: 'let fn = a => b => c => "hello, world!"',
        options: {
          minify: true,
          target: 'ES5'
        }
      });

      expect(compiled).toBe(
        `"use strict";var fn=function fn(a){return function(b){return function(c){return"hello, world!"}}};`
      );
    });
  });
});
