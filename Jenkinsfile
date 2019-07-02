pipeline {
  agent any
  options { disableConcurrentBuilds() }
  stages {
    stage('Prepare') {
      steps {
        sh '''rustup install nightly-2018-12-24'''
        sh '''rustup default nightly-2018-12-24'''
        sh '''rustup component add rustfmt-preview'''
      }
    }
    stage('Build') {
      steps {
        // Looks like g++-7 is not compatible with Xenial's Boost:
        // /usr/include/boost/multiprecision/cpp_int.hpp:181:4: error: right operand of shift expression ‘(1 << 63)’ is >= than the precision of the left operand [-fpermissive]
        // We have to point CMake back at g++-5 for now.
        sh '''
export CC=gcc-5 CXX=g++-5
cargo build --features native -vv --color never 2>&1 | grep --line-buffered -v '     Running `rustc --' | grep --line-buffered -v '       Fresh'
cargo test --features native
'''
      }
    }
    stage('Test Trade') {
      steps {
        // --nocapture here allows us to examine the trade logs even when the trade was successful.
        sh '''cargo test --features native trade_pizza_etomic -- --ignored --nocapture'''
        sh '''cargo test --features native trade_etomic_pizza -- --ignored --nocapture'''
      }
    }
  }
}