use std::ops::Div;

use crate::{
    errors::OptionError,
    state::{Lp, OptionDetail, User},
    utils::SOL_PRICE_ID,
};
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, Transfer as SplTransfer},
};
use pyth_solana_receiver_sdk::price_update::{get_feed_id_from_hex, PriceUpdateV2};

pub fn sell_option(
    ctx: Context<SellOption>,
    amount: u64,
    strike: f64,
    period: f64,
    option_index: u64,
    is_call: bool, // true : call option, false : put option
    pay_sol: bool, // true : sol, false : usdc
) -> Result<()> {
    let signer = &ctx.accounts.signer;
    let signer_ata_wsol = &mut ctx.accounts.signer_ata_wsol;
    let signer_ata_usdc = &mut ctx.accounts.signer_ata_usdc;
    let lp_ata_wsol = &mut ctx.accounts.lp_ata_wsol;
    let lp_ata_usdc = &mut ctx.accounts.lp_ata_usdc;
    let lp = &mut ctx.accounts.lp;
    let token_program = &ctx.accounts.token_program;
    let option_detail = &mut ctx.accounts.option_detail;
    let user = &mut ctx.accounts.user;

    require_eq!(
        option_index,
        user.option_index + 1,
        OptionError::InvalidOptionIndexError
    );

    let price_update = &mut ctx.accounts.price_update;
    let feed_id: [u8; 32] = get_feed_id_from_hex(SOL_PRICE_ID)?;
    let price = price_update.get_price_no_older_than(&Clock::get()?, 30, &feed_id)?;

    let oracle_price = (price.price as f64) * 10f64.powi(price.exponent);

    //calc premium
    let period_sqrt = period.sqrt(); // Using floating-point sqrt
    let iv = 0.6;
    let premium = period_sqrt
        * iv
        * if is_call {
            // call - covered sol option
            oracle_price / strike
        } else {
            // put - cash secured usdc option
            strike / oracle_price
        };
    let premium_sol = premium.div(oracle_price) as u64;
    if pay_sol {
        require_gte!(
            signer_ata_wsol.amount,
            premium_sol,
            OptionError::InvalidSignerBalanceError
        );
        token::transfer(
            CpiContext::new(
                token_program.to_account_info(),
                SplTransfer {
                    from: signer_ata_wsol.to_account_info(),
                    to: lp_ata_wsol.to_account_info(),
                    authority: signer.to_account_info(),
                },
            ),
            premium_sol,
        )?;
    } else {
        require_gte!(
            signer_ata_usdc.amount,
            premium as u64,
            OptionError::InvalidSignerBalanceError
        );
        // send premium to pool
        token::transfer(
            CpiContext::new(
                token_program.to_account_info(),
                SplTransfer {
                    from: signer_ata_usdc.to_account_info(),
                    to: lp_ata_usdc.to_account_info(),
                    authority: signer.to_account_info(),
                },
            ),
            premium as u64,
        )?;
    }
    // Lock assets for call(covered sol)/ put(secured-cash usdc) option
    if is_call {
        require_gte!(lp.sol_amount, amount, OptionError::InvalidPoolBalanceError);
        lp.sol_amount += premium as u64;
        lp.locked_sol_amount += premium as u64;
        lp.sol_amount -= premium as u64;
    } else {
        require_gte!(lp.usdc_amount, amount, OptionError::InvalidPoolBalanceError);
        lp.usdc_amount += premium as u64;
        lp.locked_usdc_amount += premium as u64;
        lp.usdc_amount -= premium as u64;
    }

    // store option data
    option_detail.index = option_index;
    option_detail.sol_amount = amount;
    option_detail.expired_date = period as u64;
    option_detail.strike_price = strike;
    option_detail.premium = premium as u64;
    option_detail.premium_unit = pay_sol;
    option_detail.option_type = is_call;
    option_detail.valid = true;
    user.option_index = option_index;

    Ok(())
}

#[derive(Accounts)]
#[instruction(option_index: u64)]
pub struct SellOption<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    pub wsol_mint: Account<'info, Mint>,
    pub usdc_mint: Account<'info, Mint>,

    #[account(
    mut,
    associated_token::mint = wsol_mint,
    associated_token::authority = signer,
  )]
    pub signer_ata_wsol: Box<Account<'info, TokenAccount>>,

    #[account(
      mut,
      associated_token::mint = usdc_mint,
      associated_token::authority = signer,
    )]
    pub signer_ata_usdc: Box<Account<'info, TokenAccount>>,

    #[account(
    seeds = [b"lp"],
    bump,
  )]
    pub lp: Box<Account<'info, Lp>>,

    #[account(
    associated_token::mint = wsol_mint,
    associated_token::authority = lp,
  )]
    pub lp_ata_wsol: Box<Account<'info, TokenAccount>>,

    #[account(
      associated_token::mint = wsol_mint,
      associated_token::authority = lp,
    )]
    pub lp_ata_usdc: Box<Account<'info, TokenAccount>>,

    #[account(
    init_if_needed,
    payer = signer,
    space=User::LEN,
    seeds = [b"user", signer.key().as_ref()],
    bump,
  )]
    pub user: Box<Account<'info, User>>,

    #[account(
      init,
      payer = signer,
      space=OptionDetail::LEN,
      seeds = [b"option", signer.key().as_ref(), &option_index.to_le_bytes()[..]],
      bump,
    )]
    pub option_detail: Box<Account<'info, OptionDetail>>,
    pub price_update: Account<'info, PriceUpdateV2>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
