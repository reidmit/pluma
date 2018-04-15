import CompilerError from './compiler-error';

function fail(message, fileName) {
  throw new CompilerError(message, fileName);
}

function generate({ asts, entryExports, options = {} }) {
  let indent = 0;
  let output = '';

  if (!asts || !asts.length) {
    fail('No syntax trees provided to code generation step.');
  }

  function generateIndent() {
    for (let i = indent; i > 0; i--) {
      output += '  ';
    }
  }

  function generateIdentifier(node) {
    output += node.value;
  }

  function generateArray(node) {
    output += '[';
    node.elements.forEach((element, i) => {
      generateNode(element);
      if (i < node.elements.length - 1) output += ', ';
    });
    output += ']';
  }

  function generateAssignment(node) {
    generateIndent();
    generateIdentifier(node.id);
    output += ' = ';
    generateNode(node.value);
    output += ';\n\n';
  }

  function generateBoolean(node) {
    output += node.value;
  }

  function generateFunction(node) {
    output += 'function(';
    generateIdentifier(node.parameter);
    output += ') {\n';
    indent++;
    generateIndent();
    output += 'return ';
    generateNode(node.body);
    output += ';\n';
    indent--;
    generateIndent();
    output += '}';
  }

  function generateString(node) {
    output += '"' + node.value + '"';
  }

  function generateNumber(node) {
    output += node.value + '';
  }

  function generateRecord(node) {
    output += '{ ';
    node.properties.forEach((prop, i) => {
      generateIdentifier(prop.key);
      output += ': ';
      generateNode(prop.value);
      if (i < node.properties.length - 1) output += ', ';
    });
    output += ' }';
  }

  function generateInterpolatedString(node) {
    output += '(';
    generateString(node.literals[0]);
    for (let i = 0; i < node.expressions.length; i++) {
      output += ' + ';
      generateNode(node.expressions[i]);
      const lit = node.literals[i + 1];
      if (lit.value) {
        output += ' + ';
        generateString(lit);
      }
    }
    output += ')';
  }

  function generateLetExpression(node) {
    output += '(function() {\n';
    indent++;
    generateIndent();
    const localVars = node.assignments.map(asgn => asgn.id.value).join(', ');
    output += 'var ' + localVars + ';\n';
    node.assignments.forEach(generateAssignment);
    generateIndent();
    output += 'return ';
    generateNode(node.body);
    output += ';\n';
    indent--;
    generateIndent();
    output += '})()';
  }

  function generateCall(node) {
    generateNode(node.callee);
    output += '(';
    generateNode(node.argument);
    output += ')';
  }

  function generateConditional(node) {
    output += '(';
    generateNode(node.predicate);
    output += ' ? ';
    generateNode(node.thenCase);
    output += ' : ';
    generateNode(node.elseCase);
    output += ')';
  }

  function generatePipeExpression(node) {
    generateNode(node.right);
    output += '(';
    generateNode(node.left);
    output += ')';
  }

  function generateTypeDeclaration(node) {
    node.typeConstructors.forEach(ctor => {
      generateIndent();
      output += 'var ';
      generateIdentifier(ctor.typeName);
      output += ' = ';

      if (ctor.typeParameters.length === 0) {
        output += "{ $ctor: '";
        generateIdentifier(ctor.typeName);
        output += "' };\n\n";
      } else {
        ctor.typeParameters.forEach((param, i) => {
          if (i > 0) output += 'return ';
          output += 'function(';
          generateIdentifier(param);
          output += ') {\n';
          indent++;
          generateIndent();
        });

        output += "return { $ctor: '";
        generateIdentifier(ctor.typeName);
        output += "', $args: [";
        output += ctor.typeParameters.map(p => p.value).join(', ');
        output += '] };\n';

        ctor.typeParameters.forEach((param, i) => {
          indent--;
          generateIndent();
          output += '};\n';
          if (i === ctor.typeParameters.length - 1) {
            output += '\n';
          }
        });
      }
    });
  }

  function generateNode(node) {
    switch (node.kind) {
      case 'Array':
        return generateArray(node);
      case 'Assignment':
        return generateAssignment(node);
      case 'Boolean':
        return generateBoolean(node);
      case 'Call':
        return generateCall(node);
      case 'Conditional':
        return generateConditional(node);
      case 'Function':
        return generateFunction(node);
      case 'Identifier':
        return generateIdentifier(node);
      case 'InterpolatedString':
        return generateInterpolatedString(node);
      case 'LetExpression':
        return generateLetExpression(node);
      case 'String':
        return generateString(node);
      case 'Number':
        return generateNumber(node);
      case 'PipeExpression':
        return generatePipeExpression(node);
      case 'Record':
        return generateRecord(node);
      case 'TypeDeclaration':
        return generateTypeDeclaration(node);
      default:
        throw 'No case for node of kind ' + node.kind;
    }
  }

  function generateExports(exportNodes = []) {
    generateIndent();
    output += 'return {';
    exportNodes.forEach((node, i) => {
      output += ' ';
      generateIdentifier(node);
      output += ': ';
      generateIdentifier(node);
      if (i < exportNodes.length - 1) output += ',';
      else output += ' ';
    });
    output += '};\n';
  }

  function generateModule(moduleNode) {
    const moduleName = moduleNode.moduleName || moduleNode.name.value;
    generateIndent();
    output += `var module$${moduleName} = (function() {\n`;
    indent++;

    const assignedVariableNames = moduleNode.body
      .filter(node => node.kind === 'Assignment')
      .map(node => node.id.value)
      .join(', ');

    if (assignedVariableNames) {
      generateIndent();
      output += 'var ' + assignedVariableNames + ';\n\n';
    }

    moduleNode.body.forEach(node => {
      generateNode(node);
      if (!/;[\s]*$/.test(output)) output += ';\n\n';
    });

    generateExports(moduleNode.exports);

    indent--;

    output += '})();';
  }

  output += `(function(global, factory) {
  typeof exports === 'object' && typeof module !== 'undefined' ? module.exports = factory() :
  typeof define === 'function' && define.amd ? define(factory) :
  (global.Pluma = factory());
})(this, function() { 'use strict';\n\n`;

  asts.map(generateModule);

  if (entryExports) {
    output += `\n\nreturn ${entryExports};`;
  }

  output += '\n});';

  return output;
}

export default generate;
