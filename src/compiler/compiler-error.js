class CompilerError extends Error {
  constructor(message, fileName) {
    super('\n\n' + message);
    this.name = `Compiler error${fileName ? " in '" + fileName + "'" : ''}`;
    this.stack = null;
  }
}

export default CompilerError;
