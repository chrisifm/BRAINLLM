const path=require('node:path')
const MiniCssExtractPlugin=require('mini-css-extract-plugin')
const HtmlWebpackPlugin=require('html-webpack-plugin')
module.exports={mode:'production',entry:'./src/main.jsx',output:{path:path.resolve(__dirname,'dist'),filename:'assets/[contenthash].js',clean:true},devtool:false,module:{rules:[{test:/\.jsx?$/,exclude:/node_modules/,use:{loader:'babel-loader',options:{presets:[['@babel/preset-react',{runtime:'automatic',development:false}]]}}},{test:/\.css$/,use:[MiniCssExtractPlugin.loader,'css-loader']}]},plugins:[new MiniCssExtractPlugin({filename:'assets/[contenthash].css'}),new HtmlWebpackPlugin({template:'index.html'})]}
