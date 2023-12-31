use calamine::{open_workbook, Xlsx};
use clap::Parser;
use polars::prelude::*;

// TODO: Make possiblity to analyze selected company and show which elements are not matching
// selection


/// Program to help to analyze Dividend companies (Fetch XLSX list from: https://moneyzine.com/investments/dividend-champions/)
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Data in XLSX format (Fetch from https://moneyzine.com/investments/dividend-champions/)
    #[arg(long, required = true)]
    data: String,

    /// Name of the list with companies increasing dividends. Possible values: "Champions", "Contenders", "Challengers", "All"
    #[arg(long, default_value = "Champions")]
    list: String,

    /// Symbol names of companies from dividend list as provided with "data" argument
    #[arg(long, requires = "data", default_values_t = &[] )]
    company: Vec<String>,

    /// Average USA inflation during investment time[%]
    #[arg(long, default_value_t = 3.4)]
    inflation: f64,

    /// Minimum accepted Dividend Yield[%]
    #[arg(long, default_value_t = 4.7)]
    min_div_yield: f64,

    /// Maximum accepted Dividend Yield[%]
    #[arg(long, default_value_t = 10.0)]
    max_div_yield: f64,

    /// Minimum accepted Dividend Growth rate[%]
    #[arg(long, default_value_t = 10.0)]
    min_div_growth_rate: f64,

    /// Maximum accepted Dividend Payout rate[%]
    #[arg(long, default_value_t = 75.0)]
    max_div_payout_rate: f64,

    /// Standard and Poor 500 list's average DIV Yield[%]
    #[arg(long, default_value_t = 1.61)]
    sp500_divy: f64,
}

fn analyze_div_yield(
    df: &DataFrame,
    sp500_divy: f64,
    inflation: f64,
    min_divy: f64,
    max_divy: f64,
) -> Result<DataFrame, &'static str> {
    // Dividend Yield should:
    // 1. Be higher than inflation rate
    // 2. be higher than 1.5*S&P500 Div Yield rate
    // 3. No More than 10% (over 10% is suspecious, check their cash flow)
    let min_ref_sp500 = sp500_divy * 1.5;
    let mut minimal_accepted_divy = if min_ref_sp500 > inflation {
        min_ref_sp500
    } else {
        inflation
    };
    if min_divy > minimal_accepted_divy {
        minimal_accepted_divy = min_divy;
    };

    let divy_col = df
        .column("Div Yield")
        .map_err(|_| "Div Yield column does not exist!")?;

    let mask = divy_col
        .gt(minimal_accepted_divy)
        .map_err(|_| "Could not apply filtering data based on Div Yield and Inflation Div Yield")?;
    let mask2 = divy_col
        .lt_eq(max_divy)
        .map_err(|_| "Error creating filter of min_growth_rate")?;
    let mask = mask & mask2;

    let filtred_df = df.filter(&mask).expect("Error filtering");

    filtred_df
        .sort(["Div Yield"], true, false)
        .map_err(|_| "Could not sort along 'Div Yield'")
}

fn analyze_dividend_payout_rate(
    df: &DataFrame,
    max_threshold: f64,
) -> Result<DataFrame, &'static str> {
    // Dividend Payout rate
    // 1. Is Current Div / Cash flow per share e.g. 0.22 / 1.7  = 0.129412
    // 2. No more than 75%

    let cols = df
        .columns(&["Current Div", "CF/Share"])
        .map_err(|_| "Current Div and/or CF/Share columns do not exist!")?;
    let mask = (cols[0] / cols[1])
        .lt(&Series::new("", &[max_threshold]))
        .unwrap();
    let filtred_df = df.filter(&mask).expect("Error filtering");

    filtred_df
        .sort(["Div Yield"], true, false)
        .map_err(|_| "Could not sort along 'Div Yield'")
}

fn analyze_div_growth(df: &DataFrame, min_growth_rate: f64) -> Result<DataFrame, &'static str> {
    // Dividend growth rate
    // 1. 10% min (more or less) depending on historical growth

    let min_div_growth_5y_to_10y_ratio = 1.0;

    let cols = df
        .columns(&["DGR 1Y", "DGR 3Y", "DGR 5Y", "DGR 10Y"])
        .map_err(|_| "DGR (dividend growth) columns do not exist!")?;
    let mask = (cols[2] / cols[3])
        .gt_eq(&Series::new("", &[min_div_growth_5y_to_10y_ratio]))
        .unwrap();
    let mask2 = cols[0]
        .gt_eq(min_growth_rate)
        .map_err(|_| "Error creating filter of min_growth_rate")?;
    let mask = mask & mask2;

    let filtred_df = df.filter(&mask).expect("Error filtering");

    filtred_df
        .sort(["DGR 1Y"], true, false)
        .map_err(|_| "Could not sort along 'DGR 1Y'")
}

fn print_summary(df: &DataFrame, company : Option<&str>) -> Result<(), &'static str> {

    let dfs = match company {
        Some(company) => {
           let mask = df.column("Symbol").map_err(|_| "Error: Unable to get Symbol")?
        .equal(company).map_err(|_| "Error: Unable to create mask")?;
        df.filter(&mask).map_err(|_| "Error: Unable to get Symbol")?
        }
        ,
        None => df.clone(),
    };
    if dfs.height() == 0 {
        return Err("Company symbol not present in selected List");
    }

    let mut selected_df = dfs.select(&["Symbol", "Company", "Current Div", "Div Yield", "Price"]) .map_err(|_| "Unable to select mentioned columns!")?;
    log::info!("Selected companies: {selected_df}");

    let mut rate = dfs
        .column("Annualized")
        .expect("No \"Current Div\" column")
        / dfs.column("CF/Share").expect("No \"CF/Share\" column")
        * 100.0;
    let rate = rate.rename("Div Payout Rate[%]");
    selected_df
        .with_column(rate.clone())
        .expect("Unable to add Rate column");
    println!("{selected_df}");
    Ok(())
}

fn main() -> Result<(), &'static str> {
    investments_forecasting::init_logging_infrastructure();

    let args = Args::parse();

    let mut excel: Xlsx<_> = open_workbook(args.data).map_err(|_| "Error: opening XLSX")?;

    // Champions
    let data = investments_forecasting::load_list(&mut excel, &args.list)?;


    // For no handpicked compnies just make overall analysis
    if args.company.len() == 0 {

        let data_shortlisted_dy = analyze_div_yield(
            &data,
            args.sp500_divy,
            args.inflation,
            args.min_div_yield,
            args.max_div_yield,
        )?;
        log::info!("Champions Shortlisted by DivY: {}", data_shortlisted_dy);

        let data_shortlisted_dy_dp =
            analyze_dividend_payout_rate(&data_shortlisted_dy, args.max_div_payout_rate / 100.0)?;

        log::info!(
            "Champions Shortlisted by DivY and Div Pay-Out: {}",
            data_shortlisted_dy_dp
        );

        let data_shortlisted_dy_dp_dg =
            analyze_div_growth(&data_shortlisted_dy_dp, args.min_div_growth_rate)?;

        print_summary(&data_shortlisted_dy_dp_dg,None)?;

    } else {
        args.company
            .iter()
            .try_for_each(|symbol| print_summary(&data,Some(&symbol)))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_divy() -> Result<(), String> {
        let inflation = 3.4;
        let sp500_divy = 1.61;
        let max_divy = 10.0;
        let min_divy = 3.9;

        let s1 = Series::new("Symbol", &["ABM", "INTC", "CAT"]);
        let s2 = Series::new("Div Yield", &[5.54, 1.32, 4.0]);

        let df: DataFrame = DataFrame::new(vec![s1, s2]).unwrap();

        let s1 = Series::new("Symbol", &["ABM", "CAT"]);
        let s2 = Series::new("Div Yield", &[5.54, 4.0]);

        let ref_df: DataFrame = DataFrame::new(vec![s1, s2]).unwrap();

        let result = analyze_div_yield(&df, sp500_divy, inflation, min_divy, max_divy).unwrap();
        assert!(result.frame_equal(&ref_df));
        Ok(())
    }

    #[test]
    fn test_analyze_divy_min() -> Result<(), String> {
        let inflation = 3.4;
        let sp500_divy = 1.61;
        let max_divy = 10.0;
        let min_divy = 5.0;

        let s1 = Series::new("Symbol", &["ABM", "INTC", "CAT"]);
        let s2 = Series::new("Div Yield", &[9.0, 1.32, 4.0]);

        let df: DataFrame = DataFrame::new(vec![s1, s2]).unwrap();

        let s1 = Series::new("Symbol", &["ABM"]);
        let s2 = Series::new("Div Yield", &[9.0]);

        let ref_df: DataFrame = DataFrame::new(vec![s1, s2]).unwrap();

        let result = analyze_div_yield(&df, sp500_divy, inflation, min_divy, max_divy).unwrap();
        assert!(result.frame_equal(&ref_df));
        Ok(())
    }

    #[test]
    fn test_analyze_divy_max() -> Result<(), String> {
        let inflation = 3.4;
        let sp500_divy = 1.61;
        let max_divy = 10.0;
        let min_divy = 3.0;

        let s1 = Series::new("Symbol", &["ABM", "INTC", "CAT"]);
        let s2 = Series::new("Div Yield", &[11.0, 1.32, 4.0]);

        let df: DataFrame = DataFrame::new(vec![s1, s2]).unwrap();

        let s1 = Series::new("Symbol", &["CAT"]);
        let s2 = Series::new("Div Yield", &[4.0]);

        let ref_df: DataFrame = DataFrame::new(vec![s1, s2]).unwrap();

        let result = analyze_div_yield(&df, sp500_divy, inflation, min_divy, max_divy).unwrap();
        assert!(result.frame_equal(&ref_df));
        Ok(())
    }

    #[test]
    fn test_analyze_divy_dpy() -> Result<(), String> {
        let max_payout_rate = 0.75;

        let s1 = Series::new("Symbol", &["ABM", "INTC", "CAT"]);
        let s2 = Series::new("Div Yield", &[5.54, 1.32, 4.0]);
        let s3 = Series::new("Current Div", &[0.54, 1.62, 0.14]);
        let s4 = Series::new("CF/Share", &[10.0, 2.0, 20.0]);

        let df: DataFrame = DataFrame::new(vec![s1, s2, s3, s4]).unwrap();

        let s1 = Series::new("Symbol", &["ABM", "CAT"]);
        let s2 = Series::new("Div Yield", &[5.54, 4.0]);
        let s3 = Series::new("Current Div", &[0.54, 0.14]);
        let s4 = Series::new("CF/Share", &[10.0, 20.0]);

        let ref_df: DataFrame = DataFrame::new(vec![s1, s2, s3, s4]).unwrap();
        //print!("Ref DF: {ref_df}");

        let result = analyze_dividend_payout_rate(&df, max_payout_rate).unwrap();
        //print!("result DF: {result}");
        assert!(result.frame_equal(&ref_df));
        Ok(())
    }

    #[test]
    fn test_analyze_div_growth() -> Result<(), String> {
        let min_growth_rate = 7.0;

        let s1 = Series::new("Symbol", &["ABM", "INTC", "CAT"]);
        let s2 = Series::new("Div Yield", &[5.54, 1.32, 4.0]);
        let s3 = Series::new("Current Div", &[0.54, 1.62, 0.14]);
        let s4 = Series::new("CF/Share", &[10.0, 2.0, 20.0]);
        let s5 = Series::new("DGR 1Y", &[7.05, 0.68, 3.94]);
        let s6 = Series::new("DGR 3Y", &[8.51, 0.91, 3.07]);
        let s7 = Series::new("DGR 5Y", &[8.96, 3.36, 5.29]);
        let s8 = Series::new("DGR 10Y", &[8.87, 9.34, 4.97]);

        let df: DataFrame = DataFrame::new(vec![s1, s2, s3, s4, s5, s6, s7, s8]).unwrap();

        let s1 = Series::new("Symbol", &["ABM"]);
        let s2 = Series::new("Div Yield", &[5.54]);
        let s3 = Series::new("Current Div", &[0.54]);
        let s4 = Series::new("CF/Share", &[10.0]);
        let s5 = Series::new("DGR 1Y", &[7.05]);
        let s6 = Series::new("DGR 3Y", &[8.51]);
        let s7 = Series::new("DGR 5Y", &[8.96]);
        let s8 = Series::new("DGR 10Y", &[8.87]);
        let ref_df: DataFrame = DataFrame::new(vec![s1, s2, s3, s4, s5, s6, s7, s8]).unwrap();
        //print!("Ref DF: {ref_df}");

        let result = analyze_div_growth(&df, min_growth_rate).unwrap();
        //        print!("result DF: {result}");
        assert!(result.frame_equal(&ref_df));
        Ok(())
    }
}
